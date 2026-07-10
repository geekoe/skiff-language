use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use skiff_runtime_request::{
    cancellation::CancellationToken, OutboundRequestLease, OutboundResponse, RequestEnvelope,
    RuntimeOperation,
};
use skiff_runtime_transport::{
    control_response_mapper::{spawn_claim_response_payload_bytes, SpawnClaimControlResponse},
    protocol::{
        encode_binary_frame, SpawnClaimDescriptorFrameMetadata, SpawnClaimRequestFrameHeader,
        SpawnCompleteRequestFrameHeader, SpawnCompleteResponseFrameHeader,
        SpawnFailRequestFrameHeader, SpawnFailResponseFrameHeader, SpawnRenewRequestFrameHeader,
        SpawnRenewResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{
    sync::{mpsc, oneshot, Notify},
    task::JoinHandle,
    time::{sleep, timeout, Duration},
};
use tracing::{error, warn};

use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
};

use super::{request_supervisor::CompletionTrace, ServiceRuntimeContext};

const CLAIM_CONTROL_TARGET: &str = "spawn.claim";
const RENEW_CONTROL_TARGET: &str = "spawn.renew";
const COMPLETE_CONTROL_TARGET: &str = "spawn.complete";
const FAIL_CONTROL_TARGET: &str = "spawn.fail";
const CONTROL_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const SPAWN_RENEW_INTERVAL: Duration = Duration::from_secs(10);
const EMPTY_CLAIM_BACKOFF_MIN: Duration = Duration::from_millis(100);
const EMPTY_CLAIM_BACKOFF_MAX: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct SpawnWorkerRegistry {
    workers: StdMutex<HashMap<String, Vec<SpawnWorkerHandle>>>,
}

struct SpawnWorkerHandle {
    worker_id: String,
    stop: Arc<SpawnWorkerStop>,
    join: JoinHandle<()>,
}

impl SpawnWorkerRegistry {
    fn register(&self, build_id: String, handle: SpawnWorkerHandle) {
        if let Ok(mut workers) = self.workers.lock() {
            workers.entry(build_id).or_default().push(handle);
        }
    }

    pub(super) async fn stop_builds(&self, build_ids: &[String]) -> usize {
        let mut handles = Vec::new();
        if let Ok(mut workers) = self.workers.lock() {
            for build_id in build_ids {
                if let Some(build_handles) = workers.remove(build_id) {
                    handles.extend(build_handles);
                }
            }
        }
        let count = handles.len();
        for handle in &handles {
            handle.stop.request_stop();
        }
        for handle in handles {
            if let Err(error) = handle.join.await {
                warn!(
                    event = "runtime.spawn_worker_join_error",
                    worker_id = %handle.worker_id,
                    error = %error
                );
            }
        }
        count
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn worker_count_for_build(&self, build_id: &str) -> usize {
        self.workers
            .lock()
            .map(|workers| workers.get(build_id).map(Vec::len).unwrap_or(0))
            .unwrap_or(0)
    }
}

struct SpawnWorkerStop {
    stopped: AtomicBool,
    notify: Notify,
}

impl SpawnWorkerStop {
    fn new() -> Self {
        Self {
            stopped: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    fn request_stop(&self) {
        if !self.stopped.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    async fn notified(&self) {
        if self.is_stopped() {
            return;
        }
        self.notify.notified().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClaimOutcome {
    Claimed,
    Empty,
}

pub(super) fn start_spawn_workers(
    host: super::RuntimeHost,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
) -> usize {
    start_spawn_workers_for_services(host.clone(), sender, host.service_snapshot())
}

pub(super) fn start_spawn_workers_for_services(
    host: super::RuntimeHost,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    services: Vec<Arc<ServiceRuntimeContext>>,
) -> usize {
    let mut started = 0;
    for service in services
        .into_iter()
        .filter(|service| !service.linked_image.spawn_routes.is_empty())
    {
        let worker_id = format!("spawn-worker-{}", uuid::Uuid::new_v4());
        let stop = Arc::new(SpawnWorkerStop::new());
        let build_id = service.build_id.clone();
        let worker = SpawnWorker {
            host: host.clone(),
            service,
            sender: sender.clone(),
            worker_id: worker_id.clone(),
            renew_interval: SPAWN_RENEW_INTERVAL,
            stop: stop.clone(),
        };
        let join = tokio::spawn(async move { worker.run().await });
        host.spawn_workers.register(
            build_id,
            SpawnWorkerHandle {
                worker_id,
                stop,
                join,
            },
        );
        started += 1;
    }
    started
}

#[derive(Clone)]
struct SpawnWorker {
    host: super::RuntimeHost,
    service: Arc<ServiceRuntimeContext>,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    worker_id: String,
    renew_interval: Duration,
    stop: Arc<SpawnWorkerStop>,
}

impl SpawnWorker {
    async fn run(self) {
        let mut backoff = EMPTY_CLAIM_BACKOFF_MIN;
        while !self.sender.is_closed() && !self.stop.is_stopped() {
            match self.claim_once().await {
                Ok(ClaimOutcome::Claimed) => {
                    backoff = EMPTY_CLAIM_BACKOFF_MIN;
                }
                Ok(ClaimOutcome::Empty) => {
                    if self.sleep_or_stop(backoff).await {
                        break;
                    }
                    backoff = (backoff * 2).min(EMPTY_CLAIM_BACKOFF_MAX);
                }
                Err(error) => {
                    if self.stop.is_stopped() {
                        break;
                    }
                    warn!(
                        event = "runtime.spawn_worker_error",
                        runtime_id = %self.service.runtime_id,
                        service_id = %self.service.service_id,
                        worker_id = %self.worker_id,
                        error = %error
                    );
                    if self.sleep_or_stop(backoff).await {
                        break;
                    }
                    backoff = (backoff * 2).min(EMPTY_CLAIM_BACKOFF_MAX);
                }
            }
        }
    }

    async fn sleep_or_stop(&self, duration: Duration) -> bool {
        tokio::select! {
            _ = sleep(duration) => false,
            _ = self.stop.notified() => true,
        }
    }

    async fn claim_once(&self) -> Result<ClaimOutcome> {
        if self.stop.is_stopped() {
            return Ok(ClaimOutcome::Empty);
        }
        let claim = self.claim_spawn().await?;
        let Some((descriptor, payload_bytes)) = claim else {
            return Ok(ClaimOutcome::Empty);
        };

        let execution_result = self
            .execute_claimed_function(&descriptor, payload_bytes)
            .await;
        match execution_result {
            Ok(()) => {
                self.complete_spawn(&descriptor).await?;
            }
            Err(error) => {
                let diagnostics = diagnostics_for_error(&error);
                if let Err(fail_error) = self.fail_spawn(&descriptor, diagnostics).await {
                    warn!(
                        event = "runtime.spawn_fail_report_error",
                        runtime_id = %self.service.runtime_id,
                        service_id = %self.service.service_id,
                        item_id = %descriptor.item_id,
                        lease_id = %descriptor.lease_id,
                        execution_error = %error,
                        fail_error = %fail_error
                    );
                    return Err(fail_error);
                }
            }
        }
        Ok(ClaimOutcome::Claimed)
    }

    async fn claim_spawn(&self) -> Result<Option<(SpawnClaimDescriptorFrameMetadata, Vec<u8>)>> {
        let header = SpawnClaimRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.claim.request".to_string(),
            rpc_id: self.control_rpc_id(CLAIM_CONTROL_TARGET),
            runtime_id: self.service.runtime_id.clone(),
            worker_id: self.worker_id.clone(),
            service_id: self.service.service_id.clone(),
            service_version: self.service.service_version().to_string(),
            service_protocol_identity: self.service.contract_identity.clone(),
            supported_targets: self.supported_targets(),
            supported_spawn_compatibility_keys: self.supported_spawn_compatibility_keys(),
            build_id: Some(self.service.build_id.clone()),
            max_execution_ms: None,
            max_concurrency: Some(1.0),
        };
        let response: SpawnClaimControlResponse = self
            .send_control_request(CLAIM_CONTROL_TARGET, header, Vec::new())
            .await?;
        if !response.header.claimed {
            return Ok(None);
        }
        let payload_bytes = spawn_claim_response_payload_bytes(&response)
            .map_err(|message| RuntimeError::decode_target(CLAIM_CONTROL_TARGET, message))?;
        let descriptor = response.header.item.ok_or_else(|| RuntimeError::Protocol {
            target: CLAIM_CONTROL_TARGET.to_string(),
            message: "spawn.claim.response claimed=true missing item".to_string(),
        })?;
        Ok(Some((descriptor, payload_bytes)))
    }

    async fn complete_spawn(&self, descriptor: &SpawnClaimDescriptorFrameMetadata) -> Result<()> {
        let header = SpawnCompleteRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.complete.request".to_string(),
            rpc_id: self.control_rpc_id(COMPLETE_CONTROL_TARGET),
            runtime_id: self.service.runtime_id.clone(),
            item_id: descriptor.item_id.clone(),
            lease_id: descriptor.lease_id.clone(),
            diagnostics: None,
        };
        let _: SpawnCompleteResponseFrameHeader = self
            .send_control_request(COMPLETE_CONTROL_TARGET, header, Vec::new())
            .await?;
        Ok(())
    }

    async fn fail_spawn(
        &self,
        descriptor: &SpawnClaimDescriptorFrameMetadata,
        diagnostics: serde_json::Map<String, Value>,
    ) -> Result<()> {
        let header = SpawnFailRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.fail.request".to_string(),
            rpc_id: self.control_rpc_id(FAIL_CONTROL_TARGET),
            runtime_id: self.service.runtime_id.clone(),
            item_id: descriptor.item_id.clone(),
            lease_id: descriptor.lease_id.clone(),
            reason: "failed".to_string(),
            diagnostics: Some(diagnostics),
        };
        let _: SpawnFailResponseFrameHeader = self
            .send_control_request(FAIL_CONTROL_TARGET, header, Vec::new())
            .await?;
        Ok(())
    }

    async fn renew_spawn(
        &self,
        descriptor: &SpawnClaimDescriptorFrameMetadata,
    ) -> Result<SpawnRenewResponseFrameHeader> {
        let header = SpawnRenewRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.renew.request".to_string(),
            rpc_id: self.control_rpc_id(RENEW_CONTROL_TARGET),
            runtime_id: self.service.runtime_id.clone(),
            item_id: descriptor.item_id.clone(),
            lease_id: descriptor.lease_id.clone(),
            worker_id: self.worker_id.clone(),
        };
        let response: SpawnRenewResponseFrameHeader = self
            .send_control_request(RENEW_CONTROL_TARGET, header, Vec::new())
            .await?;
        if response.item_id != descriptor.item_id {
            return Err(RuntimeError::Protocol {
                target: RENEW_CONTROL_TARGET.to_string(),
                message: format!(
                    "spawn renew response itemId {} does not match requested item {}",
                    response.item_id, descriptor.item_id
                ),
            });
        }
        if !response.renewed {
            return Err(RuntimeError::ProviderUnavailable {
                target: RENEW_CONTROL_TARGET.to_string(),
                reason: format!(
                    "spawn lease was not renewed for item {}",
                    descriptor.item_id
                ),
            });
        }
        Ok(response)
    }

    async fn renew_spawn_loop(
        self,
        mut descriptor: SpawnClaimDescriptorFrameMetadata,
        cancellation: CancellationToken,
        mut stop_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        loop {
            tokio::select! {
                _ = &mut stop_rx => return Ok(()),
                _ = sleep(self.renew_interval_for(&descriptor)) => {
                    match self.renew_spawn(&descriptor).await {
                        Ok(response) => {
                            descriptor.lease_expires_at = response.lease_expires_at;
                        }
                        Err(error) => {
                        cancellation.cancel();
                        return Err(error);
                        }
                    }
                }
            }
        }
    }

    async fn execute_claimed_function(
        &self,
        descriptor: &SpawnClaimDescriptorFrameMetadata,
        payload_bytes: Vec<u8>,
    ) -> Result<()> {
        self.validate_claim_descriptor(descriptor)?;
        let request = RequestEnvelope {
            request_id: descriptor.runtime_request_id.clone(),
            mode: "unary".to_string(),
            target: descriptor.target.clone(),
            operation_abi_id: None,
            selector: None,
            service_id: Some(descriptor.service_id.clone()),
            build_id: self.service.build_id.clone(),
            service_protocol_identity: descriptor.service_protocol_identity.clone(),
            contract_identity: None,
            activation_identity: self.service.activation_identity.clone(),
            binary_http: None,
            http_adapter: None,
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes,
            extra: spawned_request_extra(descriptor),
        };
        let service = self.service.clone();
        let addr = service
            .linked_image
            .spawn_routes
            .get(&descriptor.target)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "claimed spawn target {} is not registered for service {}",
                    descriptor.target, service.service_id
                ))
            })?;
        let operation = RuntimeOperation {
            operation_abi_id: None,
            operation: descriptor.target.clone(),
            target: descriptor.target.clone(),
            mode: "unary".to_string(),
            parameters: Vec::new(),
            service_protocol_identity: Some(descriptor.service_protocol_identity.clone()),
            extra: serde_json::Map::new(),
        };
        let build_guard = self.host.begin_build_execution(&service.build_id)?;
        let telemetry_context = self.host.request_telemetry_context(&request, &service);
        let supervised_request = self
            .host
            .request_supervisor
            .begin(&request, telemetry_context, "spawn.request.start")
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let (renew_stop_tx, renew_stop_rx) = oneshot::channel();
        let renew_task = tokio::spawn(self.clone().renew_spawn_loop(
            descriptor.clone(),
            supervised_request.cancellation_token(),
            renew_stop_rx,
        ));
        let execution_budget = supervised_request.execution_budget();

        let request_id = request.request_id.clone();
        let _build_guard = build_guard;
        let result = self
            .host
            .execute_runtime_request(
                service.clone(),
                operation,
                addr,
                request,
                cancelled,
                cancellation,
                execution_budget.clone(),
                Some(self.sender.clone()),
            )
            .await;
        let _ = renew_stop_tx.send(());
        let renew_result = match renew_task.await {
            Ok(result) => result,
            Err(error) => Err(RuntimeError::ProviderUnavailable {
                target: RENEW_CONTROL_TARGET.to_string(),
                reason: format!("spawn renew task failed to join: {error}"),
            }),
        };
        match (result, renew_result) {
            (Ok(_), Ok(())) => {
                self.host
                    .request_supervisor
                    .complete_success(
                        &supervised_request,
                        "spawn.request.end",
                        CompletionTrace::SPAWN,
                    )
                    .await;
                Ok(())
            }
            (Ok(_), Err(renew_error)) => {
                let response_error = response_error_from_runtime_error(&renew_error);
                self.host
                    .request_supervisor
                    .complete_error(
                        &supervised_request,
                        "spawn.request.error",
                        &response_error,
                        CompletionTrace::SPAWN_RENEW_ERROR,
                    )
                    .await;
                Err(renew_error)
            }
            (Err(error), renew_result) => {
                if let Err(renew_error) = renew_result {
                    warn!(
                        event = "runtime.spawn_renew_error_after_execution_error",
                        request_id = %request_id,
                        runtime_id = %service.runtime_id,
                        service_id = %service.service_id,
                        target = %descriptor.target,
                        execution_error = %error,
                        renew_error = %renew_error
                    );
                }
                error!(
                    event = "runtime.spawn_request_error",
                    request_id = %request_id,
                    runtime_id = %service.runtime_id,
                    service_id = %service.service_id,
                    target = %descriptor.target,
                    error = %error
                );
                let response_error = response_error_from_runtime_error(&error);
                self.host
                    .request_supervisor
                    .complete_error(
                        &supervised_request,
                        "spawn.request.error",
                        &response_error,
                        CompletionTrace::SPAWN,
                    )
                    .await;
                Err(error)
            }
        }
    }

    fn validate_claim_descriptor(
        &self,
        descriptor: &SpawnClaimDescriptorFrameMetadata,
    ) -> Result<()> {
        if descriptor.target_kind != "function" {
            return Err(RuntimeError::Unsupported(format!(
                "spawn worker only supports function targets, got {}",
                descriptor.target_kind
            )));
        }
        if descriptor.service_id != self.service.service_id {
            return Err(RuntimeError::Protocol {
                target: CLAIM_CONTROL_TARGET.to_string(),
                message: format!(
                    "claimed spawn serviceId {} does not match runtime service {}",
                    descriptor.service_id, self.service.service_id
                ),
            });
        }
        if descriptor.service_version != self.service.service_version() {
            return Err(RuntimeError::Protocol {
                target: CLAIM_CONTROL_TARGET.to_string(),
                message: format!(
                    "claimed spawn serviceVersion {} does not match runtime service version {}",
                    descriptor.service_version,
                    self.service.service_version()
                ),
            });
        }
        if descriptor.service_protocol_identity != self.service.contract_identity {
            return Err(RuntimeError::Protocol {
                target: CLAIM_CONTROL_TARGET.to_string(),
                message: format!(
                    "claimed spawn protocol {} does not match runtime protocol {}",
                    descriptor.service_protocol_identity, self.service.contract_identity
                ),
            });
        }
        if descriptor.build_id != self.service.build_id {
            return Err(RuntimeError::Protocol {
                target: CLAIM_CONTROL_TARGET.to_string(),
                message: format!(
                    "claimed spawn buildId {} does not match runtime buildId {}",
                    descriptor.build_id, self.service.build_id
                ),
            });
        }
        Ok(())
    }

    fn supported_targets(&self) -> Vec<String> {
        let mut targets = self
            .service
            .linked_image
            .spawn_routes
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        targets.sort();
        targets
    }

    fn supported_spawn_compatibility_keys(&self) -> Vec<String> {
        self.supported_targets()
            .into_iter()
            .map(|target| {
                format!(
                    "{}:{}:{}",
                    self.service.service_version(),
                    self.service.contract_identity,
                    target
                )
            })
            .collect()
    }

    async fn send_control_request<THeader, TResponse>(
        &self,
        target: &str,
        header: THeader,
        payload: Vec<u8>,
    ) -> Result<TResponse>
    where
        THeader: Serialize + ControlRequestHeader,
        TResponse: DeserializeOwned,
    {
        let rpc_id = header.rpc_id().to_string();
        let frame = encode_binary_frame(&header, &payload)
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        let (response_rx, lease) = self.open_outbound_response_lease(&rpc_id)?;
        if let Err(error) = self.send_frame(&rpc_id, frame) {
            lease.cancel("runtime_disconnect");
            return Err(error);
        }

        let payload = self
            .await_control_response(target, lease, response_rx)
            .await?;
        serde_json::from_slice(&payload).map_err(|error| {
            RuntimeError::decode_target(
                target,
                format!("control response payload is not valid JSON: {error}"),
            )
        })
    }

    fn open_outbound_response_lease(
        &self,
        rpc_id: &str,
    ) -> Result<(super::OutboundResponseReceiver, OutboundRequestLease)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let lease = self.host.outbound_requests.insert_with_lease(
            rpc_id.to_string(),
            sender,
            None,
            "caller_cancel",
        )?;
        Ok((receiver, lease))
    }

    fn send_frame(&self, rpc_id: &str, frame: Vec<u8>) -> Result<()> {
        self.sender
            .send(super::RouterWriterMessage::Binary(frame))
            .map_err(|_| RuntimeError::ProviderUnavailable {
                target: rpc_id.to_string(),
                reason: "router writer channel closed".to_string(),
            })
    }

    async fn await_control_response(
        &self,
        target: &str,
        lease: OutboundRequestLease,
        mut receiver: super::OutboundResponseReceiver,
    ) -> Result<Vec<u8>> {
        match timeout(CONTROL_RPC_TIMEOUT, receiver.recv()).await {
            Ok(Some(OutboundResponse::End { payload })) => {
                lease.complete();
                Ok(payload)
            }
            Ok(Some(OutboundResponse::Error(error))) => {
                lease.complete();
                Err(RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: error.message,
                })
            }
            Ok(Some(other)) => {
                lease.cancel("unexpected_control_response");
                Err(RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: format!("control RPC received {}", other.kind()),
                })
            }
            Ok(None) => {
                lease.cancel("response_channel_closed");
                Err(RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "control response channel closed".to_string(),
                })
            }
            Err(_) => {
                lease.cancel("timeout");
                Err(RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "control response timed out".to_string(),
                })
            }
        }
    }

    fn control_rpc_id(&self, target: &str) -> String {
        format!("{}:{}:{}", self.worker_id, target, uuid::Uuid::new_v4())
    }

    fn renew_interval_for(&self, descriptor: &SpawnClaimDescriptorFrameMetadata) -> Duration {
        let Some(lease_expires_at) = &descriptor.lease_expires_at else {
            return self.renew_interval;
        };
        let Ok(expires_at) = OffsetDateTime::parse(lease_expires_at, &Rfc3339) else {
            return self.renew_interval;
        };
        let remaining_ms = (expires_at - OffsetDateTime::now_utc()).whole_milliseconds();
        if remaining_ms <= 2 {
            return Duration::from_millis(1);
        }
        let half_remaining_ms = (remaining_ms / 2).max(1) as u128;
        let fallback_ms = self.renew_interval.as_millis().max(1);
        Duration::from_millis(half_remaining_ms.min(fallback_ms) as u64)
    }
}

trait ControlRequestHeader {
    fn rpc_id(&self) -> &str;
}

impl ControlRequestHeader for SpawnClaimRequestFrameHeader {
    fn rpc_id(&self) -> &str {
        &self.rpc_id
    }
}

impl ControlRequestHeader for SpawnRenewRequestFrameHeader {
    fn rpc_id(&self) -> &str {
        &self.rpc_id
    }
}

impl ControlRequestHeader for SpawnCompleteRequestFrameHeader {
    fn rpc_id(&self) -> &str {
        &self.rpc_id
    }
}

impl ControlRequestHeader for SpawnFailRequestFrameHeader {
    fn rpc_id(&self) -> &str {
        &self.rpc_id
    }
}

fn spawned_request_extra(
    descriptor: &SpawnClaimDescriptorFrameMetadata,
) -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "caller".to_string(),
        json!({
            "kind": "spawn",
            "target": descriptor.target,
            "spawnId": descriptor.spawn_id,
            "itemId": descriptor.item_id,
            "spawnExecutionId": descriptor.spawn_execution_id
        }),
    );
    extra.insert(
        "serviceId".to_string(),
        Value::String(descriptor.service_id.clone()),
    );
    extra.insert(
        "spawn".to_string(),
        json!({
            "itemId": descriptor.item_id,
            "leaseId": descriptor.lease_id,
            "spawnId": descriptor.spawn_id,
            "spawnExecutionId": descriptor.spawn_execution_id,
            "targetKind": descriptor.target_kind,
            "payloadSchemaIdentity": descriptor.payload_schema_identity
        }),
    );
    extra
}

fn diagnostics_for_error(error: &RuntimeError) -> serde_json::Map<String, Value> {
    let mut diagnostics = serde_json::Map::new();
    diagnostics.insert(
        "error".to_string(),
        serde_json::to_value(error.payload()).unwrap_or_else(|_| {
            json!({
                "code": "RuntimeError",
                "message": error.to_string()
            })
        }),
    );
    diagnostics
}

#[cfg(test)]
pub(super) async fn claim_once_for_test(
    host: super::RuntimeHost,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    service: Arc<ServiceRuntimeContext>,
    worker_id: String,
) -> Result<ClaimOutcome> {
    SpawnWorker {
        host,
        service,
        sender,
        worker_id,
        renew_interval: SPAWN_RENEW_INTERVAL,
        stop: Arc::new(SpawnWorkerStop::new()),
    }
    .claim_once()
    .await
}

#[cfg(test)]
pub(super) async fn renew_once_for_test(
    host: super::RuntimeHost,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    service: Arc<ServiceRuntimeContext>,
    worker_id: String,
    descriptor: SpawnClaimDescriptorFrameMetadata,
) -> Result<()> {
    SpawnWorker {
        host,
        service,
        sender,
        worker_id,
        renew_interval: SPAWN_RENEW_INTERVAL,
        stop: Arc::new(SpawnWorkerStop::new()),
    }
    .renew_spawn(&descriptor)
    .await
    .map(|_| ())
}

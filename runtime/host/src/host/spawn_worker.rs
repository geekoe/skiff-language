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
        encode_binary_frame, ActivationIdentityFrameMetadata, SpawnClaimDescriptorFrameMetadata,
        SpawnClaimRequestFrameHeader, SpawnCompleteRequestFrameHeader,
        SpawnCompleteResponseFrameHeader, SpawnFailRequestFrameHeader,
        SpawnFailResponseFrameHeader, SpawnRenewRequestFrameHeader, SpawnRenewResponseFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
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
const SPAWN_WORKERS_PER_BUILD: usize = 4;

#[derive(Default)]
pub(crate) struct SpawnWorkerRegistry {
    registrations: StdMutex<HashMap<String, SpawnWorkerRegistrationState>>,
}

#[derive(Default)]
struct SpawnWorkerRegistrationState {
    builds: HashMap<String, SpawnWorkerBuild>,
}

#[derive(Default)]
struct SpawnWorkerBuild {
    workers: Vec<SpawnWorkerHandle>,
    wake: Arc<Notify>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpawnWorkerRegistration {
    id: String,
}

struct SpawnWorkerHandle {
    worker_id: String,
    stop: Arc<SpawnWorkerStop>,
    join: JoinHandle<()>,
}

impl SpawnWorkerRegistry {
    fn register_session(&self) -> SpawnWorkerRegistration {
        let registration = SpawnWorkerRegistration {
            id: format!("spawn-session-{}", uuid::Uuid::new_v4()),
        };
        if let Ok(mut registrations) = self.registrations.lock() {
            registrations.insert(
                registration.id.clone(),
                SpawnWorkerRegistrationState::default(),
            );
        }
        registration
    }

    pub(crate) fn wake_build(&self, build_id: &str) {
        let wakes = self
            .registrations
            .lock()
            .map(|registrations| {
                registrations
                    .values()
                    .filter_map(|registration| {
                        registration
                            .builds
                            .get(build_id)
                            .map(|build| build.wake.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for wake in wakes {
            wake.notify_one();
        }
    }

    pub(crate) async fn stop_registration(&self, registration: &SpawnWorkerRegistration) -> usize {
        let handles = self
            .registrations
            .lock()
            .ok()
            .and_then(|mut registrations| registrations.remove(&registration.id))
            .map(|registration| {
                registration
                    .builds
                    .into_values()
                    .flat_map(|build| build.workers)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        stop_worker_handles(handles).await
    }

    pub(super) async fn stop_builds(&self, build_ids: &[String]) -> usize {
        let mut handles = Vec::new();
        if let Ok(mut registrations) = self.registrations.lock() {
            for registration in registrations.values_mut() {
                for build_id in build_ids {
                    if let Some(build) = registration.builds.remove(build_id) {
                        handles.extend(build.workers);
                    }
                }
            }
        }
        stop_worker_handles(handles).await
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn worker_count_for_build(&self, build_id: &str) -> usize {
        self.registrations
            .lock()
            .map(|registrations| {
                registrations
                    .values()
                    .filter_map(|registration| registration.builds.get(build_id))
                    .map(|build| build.workers.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn wake_signal_for_test(
        &self,
        registration: &SpawnWorkerRegistration,
        build_id: &str,
    ) -> Option<Arc<Notify>> {
        self.registrations
            .lock()
            .ok()
            .and_then(|mut registrations| {
                registrations.get_mut(&registration.id).map(|registration| {
                    registration
                        .builds
                        .entry(build_id.to_string())
                        .or_default()
                        .wake
                        .clone()
                })
            })
    }

    #[cfg(test)]
    pub(crate) fn registration_for_test(&self) -> SpawnWorkerRegistration {
        self.register_session()
    }

    #[cfg(test)]
    pub(crate) fn registration_count_for_test(&self) -> usize {
        self.registrations
            .lock()
            .map(|registrations| registrations.len())
            .unwrap_or(0)
    }
}

async fn stop_worker_handles(handles: Vec<SpawnWorkerHandle>) -> usize {
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
        self.notified_with_after_check(|| {}).await;
    }

    async fn notified_with_after_check(&self, after_check: impl FnOnce()) {
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_stopped() {
            return;
        }
        after_check();
        notified.await;
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
) -> SpawnWorkerRegistration {
    let registration = host.spawn_workers.register_session();
    start_spawn_workers_for_services(host.clone(), sender, host.service_snapshot(), &registration);
    registration
}

pub(super) fn start_spawn_workers_for_services(
    host: super::RuntimeHost,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    services: Vec<Arc<ServiceRuntimeContext>>,
    registration: &SpawnWorkerRegistration,
) -> usize {
    let mut started = 0;
    for service in services
        .into_iter()
        .filter(|service| !service.linked_image.spawn_routes.is_empty())
    {
        let build_id = service.build_id.clone();
        let Ok(mut registrations) = host.spawn_workers.registrations.lock() else {
            continue;
        };
        let Some(registration_state) = registrations.get_mut(&registration.id) else {
            continue;
        };
        let build = registration_state.builds.entry(build_id).or_default();
        let wake = build.wake.clone();
        for _ in 0..SPAWN_WORKERS_PER_BUILD {
            let worker_id = format!("spawn-worker-{}", uuid::Uuid::new_v4());
            let stop = Arc::new(SpawnWorkerStop::new());
            let worker = SpawnWorker {
                host: host.clone(),
                service: service.clone(),
                sender: sender.clone(),
                worker_id: worker_id.clone(),
                activation_identity: None,
                renew_interval: SPAWN_RENEW_INTERVAL,
                stop: stop.clone(),
                wake: wake.clone(),
            };
            let join = tokio::spawn(async move { worker.run().await });
            build.workers.push(SpawnWorkerHandle {
                worker_id,
                stop,
                join,
            });
            started += 1;
        }
    }
    started
}

#[derive(Clone)]
struct SpawnWorker {
    host: super::RuntimeHost,
    service: Arc<ServiceRuntimeContext>,
    sender: mpsc::UnboundedSender<super::RouterWriterMessage>,
    worker_id: String,
    activation_identity: Option<ActivationIdentityFrameMetadata>,
    renew_interval: Duration,
    stop: Arc<SpawnWorkerStop>,
    wake: Arc<Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryWaitOutcome {
    Elapsed,
    Woken,
    Stopped,
}

impl SpawnWorker {
    async fn run(self) {
        if self.activation_identity.is_none() {
            warn!(
                event = "runtime.spawn_worker_missing_activation_context",
                runtime_id = %self.service.runtime_id,
                service_id = %self.service.service_id,
                "legacy service worker was not started because no pinned ActivationContext is available"
            );
            return;
        }
        let mut backoff = EMPTY_CLAIM_BACKOFF_MIN;
        while !self.sender.is_closed() && !self.stop.is_stopped() {
            match self.claim_once().await {
                Ok(ClaimOutcome::Claimed) => {
                    backoff = EMPTY_CLAIM_BACKOFF_MIN;
                }
                Ok(ClaimOutcome::Empty) => match self.wait_for_retry(backoff).await {
                    RetryWaitOutcome::Stopped => break,
                    RetryWaitOutcome::Woken => backoff = EMPTY_CLAIM_BACKOFF_MIN,
                    RetryWaitOutcome::Elapsed => {
                        backoff = (backoff * 2).min(EMPTY_CLAIM_BACKOFF_MAX)
                    }
                },
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
                    match self.wait_for_retry(backoff).await {
                        RetryWaitOutcome::Stopped => break,
                        RetryWaitOutcome::Woken => backoff = EMPTY_CLAIM_BACKOFF_MIN,
                        RetryWaitOutcome::Elapsed => {
                            backoff = (backoff * 2).min(EMPTY_CLAIM_BACKOFF_MAX)
                        }
                    }
                }
            }
        }
    }

    async fn wait_for_retry(&self, duration: Duration) -> RetryWaitOutcome {
        wait_for_retry_signal(&self.stop, &self.wake, duration).await
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
        let activation_identity = self.current_activation_identity(CLAIM_CONTROL_TARGET)?;
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
            activation_identity,
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
            activation_identity: self.current_activation_identity(COMPLETE_CONTROL_TARGET)?,
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
            activation_identity: self.current_activation_identity(FAIL_CONTROL_TARGET)?,
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
            activation_identity: self.current_activation_identity(RENEW_CONTROL_TARGET)?,
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
            ingress_selector: None,
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
        let stop_cancellation = cancellation.clone();
        let (renew_stop_tx, renew_stop_rx) = oneshot::channel();
        let renew_task = tokio::spawn(self.clone().renew_spawn_loop(
            descriptor.clone(),
            supervised_request.cancellation_token(),
            renew_stop_rx,
        ));
        let execution_budget = supervised_request.execution_budget();

        let request_id = request.request_id.clone();
        let _build_guard = build_guard;
        let execution = self.host.execute_runtime_request(
            service.clone(),
            operation,
            addr,
            request,
            cancelled,
            cancellation,
            execution_budget.clone(),
            Some(self.sender.clone()),
        );
        tokio::pin!(execution);
        let result = tokio::select! {
            result = &mut execution => result,
            _ = self.stop.notified() => {
                stop_cancellation.cancel();
                execution.await
            }
        };
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
        let expected_activation_identity =
            self.current_activation_identity(CLAIM_CONTROL_TARGET)?;
        if descriptor.activation_identity != expected_activation_identity {
            return Err(RuntimeError::Protocol {
                target: CLAIM_CONTROL_TARGET.to_string(),
                message:
                    "claimed spawn activationIdentity does not match the pinned worker activation"
                        .to_string(),
            });
        }
        Ok(())
    }

    fn current_activation_identity(
        &self,
        target: &str,
    ) -> Result<ActivationIdentityFrameMetadata> {
        self.activation_identity
            .clone()
            .ok_or_else(|| RuntimeError::Protocol {
                target: target.to_string(),
                message: "spawn control requires a current pinned ActivationContext".to_string(),
            })
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
        let response = tokio::select! {
            response = timeout(CONTROL_RPC_TIMEOUT, receiver.recv()) => response,
            _ = self.stop.notified() => {
                lease.cancel("worker_stop");
                return Err(RuntimeError::cancelled());
            }
        };
        match response {
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

async fn wait_for_retry_signal(
    stop: &SpawnWorkerStop,
    wake: &Notify,
    duration: Duration,
) -> RetryWaitOutcome {
    tokio::select! {
        _ = stop.notified() => RetryWaitOutcome::Stopped,
        _ = wake.notified() => RetryWaitOutcome::Woken,
        _ = sleep(duration) => RetryWaitOutcome::Elapsed,
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
    activation_identity: ActivationIdentityFrameMetadata,
) -> Result<ClaimOutcome> {
    SpawnWorker {
        host,
        service,
        sender,
        worker_id,
        activation_identity: Some(activation_identity),
        renew_interval: SPAWN_RENEW_INTERVAL,
        stop: Arc::new(SpawnWorkerStop::new()),
        wake: Arc::new(Notify::new()),
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
    activation_identity: ActivationIdentityFrameMetadata,
    descriptor: SpawnClaimDescriptorFrameMetadata,
) -> Result<()> {
    SpawnWorker {
        host,
        service,
        sender,
        worker_id,
        activation_identity: Some(activation_identity),
        renew_interval: SPAWN_RENEW_INTERVAL,
        stop: Arc::new(SpawnWorkerStop::new()),
        wake: Arc::new(Notify::new()),
    }
    .renew_spawn(&descriptor)
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_wake_preserves_a_permit_before_worker_waits() {
        let registry = SpawnWorkerRegistry::default();
        let registration = registry.register_session();
        let wake = registry
            .wake_signal_for_test(&registration, "build-a")
            .expect("test registration should exist");

        registry.wake_build("build-a");

        assert_eq!(
            wait_for_retry_signal(&SpawnWorkerStop::new(), &wake, Duration::from_secs(30),).await,
            RetryWaitOutcome::Woken
        );
    }

    #[tokio::test]
    async fn build_wake_interrupts_an_active_backoff() {
        let registry = Arc::new(SpawnWorkerRegistry::default());
        let registration = registry.register_session();
        let wake = registry
            .wake_signal_for_test(&registration, "build-a")
            .expect("test registration should exist");
        let wake_registry = registry.clone();
        let wake_task = tokio::spawn(async move {
            tokio::task::yield_now().await;
            wake_registry.wake_build("build-a");
        });

        assert_eq!(
            wait_for_retry_signal(&SpawnWorkerStop::new(), &wake, Duration::from_secs(30),).await,
            RetryWaitOutcome::Woken
        );
        wake_task.await.expect("wake task should finish");
    }

    #[tokio::test]
    async fn worker_stop_does_not_lose_notification_between_check_and_wait() {
        let stop = SpawnWorkerStop::new();

        timeout(
            Duration::from_millis(50),
            stop.notified_with_after_check(|| stop.request_stop()),
        )
        .await
        .expect("stop after the state check should wake the registered waiter");
    }

    #[tokio::test]
    async fn worker_stop_before_wait_returns_immediately() {
        let stop = SpawnWorkerStop::new();
        stop.request_stop();

        timeout(Duration::from_millis(50), stop.notified())
            .await
            .expect("a stop requested before waiting must be observed immediately");
    }
}

//! Durable task wire handler (`task.submit.request` / `task.status.request` /
//! `task.cancel.request`). Replaces the old volatile actor/dedicated task
//! sink: submissions become durable TaskStore records (TaskId-idempotent
//! create), immediate tasks wake the scheduler, status/cancel project
//! directly onto the store, and actor-method targets freeze their
//! `ActorActivationSnapshot` into the durable record (E2b).

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::Engine as _;
use skiff_deployment::projection::actor_routing::ActorRoutingRef;
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_task_cancel_request_frame, decode_task_status_request_frame,
    decode_task_submit_request_frame, encode_task_cancel_error_frame,
    encode_task_cancel_response_frame, encode_task_status_error_frame,
    encode_task_status_response_frame, encode_task_submit_error_frame,
    encode_task_submit_response_frame, ActorTaskRuntimeErrorFrameHeader,
    RuntimeErrorFramePayload, RuntimeFrameFamily, TaskCancelRequestFrameHeader,
    TaskCancelResultKindWire, TaskCancelResultWire, TaskCancelResponseFrameHeader,
    TaskControlRejectionCode, TaskRef, TaskStatusKindWire, TaskStatusRequestFrameHeader,
    TaskStatusResponseFrameHeader, TaskStatusWire, TaskSubmitRejectionCode,
    TaskSubmitRequestFrameHeaderV2, TaskSubmitResponseFrameHeader, TaskTargetKind,
    RUNTIME_FRAME_SCHEMA_VERSION, TASK_CANCEL_ERROR_FRAME_TYPE,
    TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED, TASK_STATUS_ERROR_FRAME_TYPE,
};
use skiff_task_control::model::{
    ActorActivationSnapshot, ActorDeclarationOwner, ActorDeclarationOwnerFile,
    ActorDeclarationOwnerUnit, DetachedCallTarget, DurableDuration, DurableUtcTimestamp,
    RecoverablePayload, ServiceOwner, TaskCancelResultKind, TaskExecutionImageRef, TaskId,
    TaskRecord, TaskState, TaskStatusKind, TaskTraceContext,
};
use skiff_task_control::scheduler::Scheduler;
use skiff_task_control::store::{CancelInput, StatusInput, TaskStore};

use crate::bootstrap::ActiveRoutingEpochStore;
use crate::session::demux::InboundFrameSink;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::TerminalKind;
use crate::supervisor::ws::WsSessionWriter;
use super::health::TaskControlCounters;
use super::project_runtime_expected_type_plan;

/// Default status/cancel retention horizon (task-control `StatusInput`).
const DEFAULT_TASK_STATUS_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Projects an authenticated `task.submit.request` into the frozen execution
/// image authority facts (authoritative design "Execution Image And Target
/// Pinning"): environment / package version / assembly / config snapshot /
/// deployment are taken from the committed routing epoch, never from the
/// Runtime's self-reported names.
pub trait TaskExecutionImageSource: Send + Sync + fmt::Debug {
    fn resolve(&self, header: &TaskSubmitRequestFrameHeaderV2) -> Option<TaskExecutionImageRef>;

    /// True when the owner scope is a service of the current routing epoch
    /// (status/cancel authority pre-check; the wire carries no caller
    /// service identity in D1, so an unknown owner is a stable expired
    /// projection).
    fn contains_service(&self, service_id: &str) -> bool;
}

/// Production image source backed by the committed routing epoch.
#[derive(Debug, Clone)]
pub struct EpochTaskExecutionImageSource {
    epoch_store: Arc<ActiveRoutingEpochStore>,
}

impl EpochTaskExecutionImageSource {
    pub fn new(epoch_store: Arc<ActiveRoutingEpochStore>) -> Self {
        Self { epoch_store }
    }
}

impl TaskExecutionImageSource for EpochTaskExecutionImageSource {
    fn resolve(&self, header: &TaskSubmitRequestFrameHeaderV2) -> Option<TaskExecutionImageRef> {
        let epoch = self.epoch_store.capture()?;
        if epoch.assembly_identity() != header.activation_identity.assembly_identity.as_str()
            || epoch.assembly_generation() != header.activation_identity.generation
        {
            return None;
        }
        let deployment = epoch.deployment_projection().iter().find(|deployment| {
            deployment.service_id == header.service_id
                && deployment.contract_version == header.service_version
                && deployment.deployment_revision.as_str()
                    == header.activation_identity.deployment_revision
        })?;
        let tuple = epoch.registered_tuple();
        Some(TaskExecutionImageRef {
            target_environment: tuple.environment.clone(),
            package_version: header.service_version.clone(),
            assembly: tuple.assembly.clone(),
            config_snapshot: tuple.config_snapshot.clone(),
            deployment: deployment.clone(),
        })
    }

    fn contains_service(&self, service_id: &str) -> bool {
        self.epoch_store
            .capture()
            .is_some_and(|epoch| {
                epoch
                    .deployment_projection()
                    .iter()
                    .any(|deployment| deployment.service_id == service_id)
            })
    }
}

/// Production `task.*` inbound sink (durable control plane).
#[derive(Clone)]
pub struct DurableTaskFrameSink {
    store: Arc<dyn TaskStore>,
    scheduler: Arc<Scheduler>,
    image_source: Arc<dyn TaskExecutionImageSource>,
    writer: Arc<dyn WsSessionWriter>,
    counters: Arc<TaskControlCounters>,
    max_payload_bytes: usize,
    retention: DurableDuration,
    seq: Arc<AtomicU64>,
}

impl std::fmt::Debug for DurableTaskFrameSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableTaskFrameSink")
            .field("max_payload_bytes", &self.max_payload_bytes)
            .field("retention", &self.retention)
            .finish_non_exhaustive()
    }
}

impl DurableTaskFrameSink {
    pub fn new(
        store: Arc<dyn TaskStore>,
        scheduler: Arc<Scheduler>,
        image_source: Arc<dyn TaskExecutionImageSource>,
        writer: Arc<dyn WsSessionWriter>,
        counters: Arc<TaskControlCounters>,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            store,
            scheduler,
            image_source,
            writer,
            counters,
            max_payload_bytes,
            retention: DurableDuration::from_millis(DEFAULT_TASK_STATUS_RETENTION_MS),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    fn write(&self, session: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), TerminalKind> {
        self.writer
            .write(session, bytes)
            .map_err(|_| TerminalKind::MalformedFrame)
    }

    fn error_frame(
        rpc_id: &str,
        code: TaskSubmitRejectionCode,
        message: &str,
    ) -> Result<Vec<u8>, TerminalKind> {
        Self::control_error_frame(rpc_id, "task.submit.error", code.as_str(), message)
    }

    /// Encodes one `task.*.error` frame (submit/status/cancel share the
    /// `ActorTaskRuntimeErrorFrameHeader` shape; the envelope type selects the
    /// exact frame family).
    fn control_error_frame(
        rpc_id: &str,
        envelope_type: &str,
        code: &str,
        message: &str,
    ) -> Result<Vec<u8>, TerminalKind> {
        let header = ActorTaskRuntimeErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: envelope_type.to_string(),
            rpc_id: rpc_id.to_string(),
            error: RuntimeErrorFramePayload {
                code: code.to_string(),
                message: message.to_string(),
                status: None,
                details: None,
            },
        };
        match envelope_type {
            "task.submit.error" => {
                encode_task_submit_error_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
            }
            TASK_STATUS_ERROR_FRAME_TYPE => {
                encode_task_status_error_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
            }
            TASK_CANCEL_ERROR_FRAME_TYPE => {
                encode_task_cancel_error_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
            }
            other => {
                debug_assert!(
                    false,
                    "control_error_frame called with unknown task envelope {other}"
                );
                Err(TerminalKind::MalformedFrame)
            }
        }
    }

    fn submit_response(rpc_id: &str, task_id: &TaskId, owner: &str) -> Result<Vec<u8>, TerminalKind> {
        let header = TaskSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "task.submit.response".to_string(),
            rpc_id: rpc_id.to_string(),
            task_ref: TaskRef::new(task_id.as_str(), owner)
                .map_err(|_| TerminalKind::MalformedFrame)?,
            task_id: task_id.as_str().to_string(),
            // D2: no execution request exists at submit time; the TaskId is
            // the stable opaque correlation carried by the response.
            request_id: task_id.as_str().to_string(),
            status: TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
        };
        encode_task_submit_response_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
    }

    /// Durable `task.submit.request` handler: validates timing/payload,
    /// creates the TaskId-idempotent record, wakes the scheduler for
    /// immediate tasks, and writes `task.submit.response` / `task.submit.error`.
    pub async fn handle_submit(
        &self,
        session: RuntimeSessionEpoch,
        header: TaskSubmitRequestFrameHeaderV2,
        payload: Vec<u8>,
    ) -> Result<(), TerminalKind> {
        let rpc_id = header.rpc_id.clone();
        let target = match resolve_target(&header) {
            Ok(target) => target,
            Err(message) => {
                self.counters
                    .submissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::error_frame(
                    &rpc_id,
                    TaskSubmitRejectionCode::Rejected,
                    &message,
                )?;
                return self.write(&session, bytes);
            }
        };
        let Some(image) = self.image_source.resolve(&header) else {
            self.counters
                .submissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::error_frame(
                &rpc_id,
                TaskSubmitRejectionCode::Rejected,
                "task activation identity does not match the active routing epoch",
            )?;
            return self.write(&session, bytes);
        };
        let Ok(now) = self.store.now().await else {
            self.counters
                .submissions_transient
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::error_frame(
                &rpc_id,
                TaskSubmitRejectionCode::StoreUnavailable,
                "task store is unavailable",
            )?;
            return self.write(&session, bytes);
        };
        let Some(due_at) = resolve_due_at(header.timing, now) else {
            self.counters
                .submissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::error_frame(
                &rpc_id,
                TaskSubmitRejectionCode::InvalidTiming,
                "task timing is negative, overflowed or unrepresentable",
            )?;
            return self.write(&session, bytes);
        };
        if payload.len() > self.max_payload_bytes {
            self.counters
                .submissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::error_frame(
                &rpc_id,
                TaskSubmitRejectionCode::QuotaExceeded,
                "task payload exceeds the submission payload quota",
            )?;
            return self.write(&session, bytes);
        }
        let task_id = header
            .task_id
            .clone()
            .map(TaskId::new)
            .unwrap_or_else(|| {
                TaskId::new(format!("task-{}", self.seq.fetch_add(1, Ordering::Relaxed)))
            });
        let record = TaskRecord {
            task_id: task_id.clone(),
            owner: ServiceOwner::new(header.service_id.clone()),
            execution: image,
            target,
            payload: RecoverablePayload::new(payload),
            due_at,
            state: TaskState::Scheduled,
            attempt_generation: 0,
            active_lease: None,
            terminal: None,
            trace: TaskTraceContext {
                trace_id: header
                    .trace_id
                    .clone()
                    .unwrap_or_else(|| format!("task-trace:{}", task_id.as_str())),
                span_id: None,
            },
            created_at: now,
            retry_not_before: None,
        };
        match self.store.create(record).await {
            Ok(_) => {
                self.counters
                    .submissions_accepted
                    .fetch_add(1, Ordering::Relaxed);
                if due_at <= now {
                    self.scheduler.wake();
                }
                let bytes =
                    Self::submit_response(&rpc_id, &task_id, &header.service_id)?;
                self.write(&session, bytes)
            }
            Err(skiff_task_control::TaskStoreError::Transient { .. })
            | Err(skiff_task_control::TaskStoreError::Closed) => {
                // Ambiguous acceptance: the durable commit may have landed.
                // Reuse the same TaskId and query; never create a second task.
                self.counters
                    .submissions_transient
                    .fetch_add(1, Ordering::Relaxed);
                let committed = matches!(
                    self.store
                        .status(StatusInput {
                            task_id: task_id.clone(),
                            retention: self.retention,
                        })
                        .await,
                    Ok(status) if status.kind != TaskStatusKind::Expired
                );
                let bytes = if committed {
                    self.counters
                        .submissions_accepted
                        .fetch_add(1, Ordering::Relaxed);
                    Self::submit_response(&rpc_id, &task_id, &header.service_id)?
                } else {
                    Self::error_frame(
                        &rpc_id,
                        TaskSubmitRejectionCode::StoreUnavailable,
                        "task store response was ambiguous and no durable task is visible",
                    )?
                };
                self.write(&session, bytes)
            }
            Err(skiff_task_control::TaskStoreError::DuplicateTaskId { .. })
            | Err(skiff_task_control::TaskStoreError::InvalidRecord { .. })
            | Err(skiff_task_control::TaskStoreError::CasMismatch { .. })
            | Err(skiff_task_control::TaskStoreError::NotFound { .. }) => {
                self.counters
                    .submissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::error_frame(
                    &rpc_id,
                    TaskSubmitRejectionCode::Rejected,
                    "task submission was rejected by the durable task store",
                )?;
                self.write(&session, bytes)
            }
        }
    }

    fn owner_is_known(&self, task_ref: &TaskRef) -> bool {
        self.image_source.contains_service(task_ref.owner())
    }

    /// Durable `task.status.request` handler (reference kind projection).
    pub async fn handle_status(
        &self,
        session: RuntimeSessionEpoch,
        request: TaskStatusRequestFrameHeader,
    ) -> Result<(), TerminalKind> {
        self.counters
            .status_queries
            .fetch_add(1, Ordering::Relaxed);
        if !self.owner_is_known(&request.task_ref) {
            // Owner scope is not a service of the active routing epoch: the
            // TaskId cannot be resolved by this caller, which is the wire
            // `notFound` projection (user surface maps it to stable expired).
            self.counters
                .status_not_found
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::control_error_frame(
                &request.rpc_id,
                TASK_STATUS_ERROR_FRAME_TYPE,
                TaskControlRejectionCode::NotFound.as_str(),
                "task reference owner scope is not a service of the active routing epoch",
            )?;
            return self.write(&session, bytes);
        }
        match self
            .store
            .status(StatusInput {
                task_id: TaskId::new(request.task_ref.task_id()),
                retention: self.retention,
            })
            .await
        {
            Ok(status) => {
                let wire = TaskStatusWire {
                    kind: wire_status_kind(status.kind),
                };
                if wire.kind == TaskStatusKindWire::Expired {
                    self.counters
                        .status_expired
                        .fetch_add(1, Ordering::Relaxed);
                }
                let header = TaskStatusResponseFrameHeader {
                    schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                    envelope_type: "task.status.response".to_string(),
                    rpc_id: request.rpc_id,
                    task_ref: request.task_ref,
                    status: wire,
                };
                let bytes = encode_task_status_response_frame(&header)
                    .map_err(|_| TerminalKind::MalformedFrame)?;
                self.write(&session, bytes)
            }
            Err(skiff_task_control::TaskStoreError::NotFound { .. }) => {
                self.counters
                    .status_not_found
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::control_error_frame(
                    &request.rpc_id,
                    TASK_STATUS_ERROR_FRAME_TYPE,
                    TaskControlRejectionCode::NotFound.as_str(),
                    "task record is not found in the durable task store",
                )?;
                self.write(&session, bytes)
            }
            Err(_) => {
                // Transient / closed / unexpected store failure: surface the
                // wire `storeUnavailable` error instead of faking a stable
                // expired projection (D2 limitation removed by E1).
                self.counters
                    .status_unavailable
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::control_error_frame(
                    &request.rpc_id,
                    TASK_STATUS_ERROR_FRAME_TYPE,
                    TaskControlRejectionCode::StoreUnavailable.as_str(),
                    "task store is unavailable",
                )?;
                self.write(&session, bytes)
            }
        }
    }

    /// Durable `task.cancel.request` handler (reference kind projection;
    /// cancel/claim races are TaskStore CAS).
    pub async fn handle_cancel(
        &self,
        session: RuntimeSessionEpoch,
        request: TaskCancelRequestFrameHeader,
    ) -> Result<(), TerminalKind> {
        if !self.owner_is_known(&request.task_ref) {
            self.counters
                .cancel_not_found
                .fetch_add(1, Ordering::Relaxed);
            let bytes = Self::control_error_frame(
                &request.rpc_id,
                TASK_CANCEL_ERROR_FRAME_TYPE,
                TaskControlRejectionCode::NotFound.as_str(),
                "task reference owner scope is not a service of the active routing epoch",
            )?;
            return self.write(&session, bytes);
        }
        match self
            .store
            .cancel(CancelInput {
                task_id: TaskId::new(request.task_ref.task_id()),
            })
            .await
        {
            Ok(result) => {
                let wire = TaskCancelResultWire {
                    kind: wire_cancel_kind(result.kind),
                };
                match wire.kind {
                    TaskCancelResultKindWire::Canceled => {
                        self.counters
                            .cancel_canceled
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    TaskCancelResultKindWire::AlreadyStarted => {
                        self.counters
                            .cancel_already_started
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    TaskCancelResultKindWire::AlreadyTerminal => {
                        self.counters
                            .cancel_already_terminal
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    TaskCancelResultKindWire::Expired => {
                        self.counters
                            .cancel_expired
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                let header = TaskCancelResponseFrameHeader {
                    schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                    envelope_type: "task.cancel.response".to_string(),
                    rpc_id: request.rpc_id,
                    task_ref: request.task_ref,
                    result: wire,
                };
                let bytes = encode_task_cancel_response_frame(&header)
                    .map_err(|_| TerminalKind::MalformedFrame)?;
                self.write(&session, bytes)
            }
            Err(skiff_task_control::TaskStoreError::NotFound { .. }) => {
                self.counters
                    .cancel_not_found
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::control_error_frame(
                    &request.rpc_id,
                    TASK_CANCEL_ERROR_FRAME_TYPE,
                    TaskControlRejectionCode::NotFound.as_str(),
                    "task record is not found in the durable task store",
                )?;
                self.write(&session, bytes)
            }
            Err(_) => {
                self.counters
                    .cancel_unavailable
                    .fetch_add(1, Ordering::Relaxed);
                let bytes = Self::control_error_frame(
                    &request.rpc_id,
                    TASK_CANCEL_ERROR_FRAME_TYPE,
                    TaskControlRejectionCode::StoreUnavailable.as_str(),
                    "task store is unavailable",
                )?;
                self.write(&session, bytes)
            }
        }
    }
}

fn resolve_due_at(
    timing: Option<skiff_runtime_transport::protocol::TaskSubmitTiming>,
    now: DurableUtcTimestamp,
) -> Option<DurableUtcTimestamp> {
    use skiff_runtime_transport::protocol::TaskSubmitTiming;
    match timing.unwrap_or(TaskSubmitTiming::Immediate) {
        TaskSubmitTiming::Immediate => Some(now),
        TaskSubmitTiming::After { duration_ms } => {
            now.checked_add_millis(i64::try_from(duration_ms).ok()?)
        }
        TaskSubmitTiming::At { utc_millis } => {
            (utc_millis >= 0).then_some(DurableUtcTimestamp::from_millis(utc_millis))
        }
    }
}

/// Projects one authenticated `task.submit.request` target into the canonical
/// `DetachedCallTarget` (E2b: actor-method targets decode their snapshot into
/// the durable record instead of being rejected as unsupported).
fn resolve_target(header: &TaskSubmitRequestFrameHeaderV2) -> Result<DetachedCallTarget, String> {
    match header.target_kind {
        TaskTargetKind::Function => Ok(DetachedCallTarget::Function {
            callable: skiff_artifact_model::PackageCallableId::new(header.target.clone()),
        }),
        TaskTargetKind::ActorMethod => {
            let actor_method = header.actor_method.as_ref().ok_or_else(|| {
                "actor-method task target is missing actorMethod metadata".to_string()
            })?;
            let key_bytes = decode_snapshot_base64(&actor_method.activation.key, "activation.key")?;
            let create_input_bytes =
                decode_snapshot_base64(&actor_method.activation.create_input, "activation.createInput")?;
            let expected_type_plan = project_runtime_expected_type_plan(
                &actor_method.activation.expected_type_plan,
            )
            .map_err(|error| format!("actor task snapshot plan is invalid: {error}"))?;
            Ok(DetachedCallTarget::ActorMethod {
                actor: ActorRoutingRef {
                    service_id: actor_method.actor_ref.service_id.clone(),
                    actor_abi_identity: actor_method.actor_abi_identity.clone(),
                },
                activation: ActorActivationSnapshot {
                    key: RecoverablePayload::new(key_bytes),
                    create_input: RecoverablePayload::new(create_input_bytes),
                    expected_type_plan,
                    expected_type_plan_runtime: Some(
                        actor_method.activation.expected_type_plan.clone(),
                    ),
                },
                implementation: actor_method.actor_implementation_identity.clone(),
                method: actor_method.method_identity.clone(),
                declaration_owner: declaration_owner_from_frame(&actor_method.declaration_owner),
            })
        }
    }
}

fn decode_snapshot_base64(value: &str, label: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| format!("task {label} is not canonical base64: {error}"))
}

fn declaration_owner_from_frame(
    owner: &ActorDeclarationOwnerFrameHeader,
) -> ActorDeclarationOwner {
    ActorDeclarationOwner {
        unit: match owner.unit {
            ActorOwnerUnitFrameHeader::Service => ActorDeclarationOwnerUnit::Service,
            ActorOwnerUnitFrameHeader::Package(slot) => ActorDeclarationOwnerUnit::Package(slot),
        },
        file: match &owner.file {
            ActorOwnerFileFrameHeader::LoadedFileIndex(index) => {
                ActorDeclarationOwnerFile::LoadedFileIndex(*index)
            }
            ActorOwnerFileFrameHeader::FileIrIdentity(identity) => {
                ActorDeclarationOwnerFile::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: owner.actor_symbol.clone(),
    }
}

fn wire_status_kind(kind: TaskStatusKind) -> TaskStatusKindWire {
    match kind {
        TaskStatusKind::Scheduled => TaskStatusKindWire::Scheduled,
        TaskStatusKind::Ready => TaskStatusKindWire::Ready,
        TaskStatusKind::Running => TaskStatusKindWire::Running,
        TaskStatusKind::Succeeded => TaskStatusKindWire::Succeeded,
        TaskStatusKind::Failed => TaskStatusKindWire::Failed,
        TaskStatusKind::PlatformFailed => TaskStatusKindWire::PlatformFailed,
        TaskStatusKind::Canceled => TaskStatusKindWire::Canceled,
        TaskStatusKind::Expired => TaskStatusKindWire::Expired,
    }
}

fn wire_cancel_kind(kind: TaskCancelResultKind) -> TaskCancelResultKindWire {
    match kind {
        TaskCancelResultKind::Canceled => TaskCancelResultKindWire::Canceled,
        TaskCancelResultKind::AlreadyStarted => TaskCancelResultKindWire::AlreadyStarted,
        TaskCancelResultKind::AlreadyTerminal => TaskCancelResultKindWire::AlreadyTerminal,
        TaskCancelResultKind::Expired => TaskCancelResultKindWire::Expired,
    }
}

impl InboundFrameSink for DurableTaskFrameSink {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Task
    }

    fn accepts_frame_type(&self, frame_type: &str) -> bool {
        matches!(
            frame_type,
            "task.submit.request" | "task.status.request" | "task.cancel.request"
        )
    }

    fn handle(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        // Codec validation stays synchronous (fail closed on malformed
        // frames); the durable store work is asynchronous and writes its
        // response through the session writer when it converges. The wire
        // correlation is rpcId/taskRef, so response order is safe.
        let this = Arc::new(self.clone());
        let session = session.clone();
        let frame_type = skiff_runtime_transport::protocol::decode_binary_frame(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?
            .header
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        match frame_type.as_str() {
            "task.submit.request" => {
                let (header, payload) =
                    decode_task_submit_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
                tokio::spawn(async move {
                    let _ = this.handle_submit(session, header, payload).await;
                });
                Ok(())
            }
            "task.status.request" => {
                let request =
                    decode_task_status_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
                tokio::spawn(async move {
                    let _ = this.handle_status(session, request).await;
                });
                Ok(())
            }
            "task.cancel.request" => {
                let request =
                    decode_task_cancel_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
                tokio::spawn(async move {
                    let _ = this.handle_cancel(session, request).await;
                });
                Ok(())
            }
            _ => Err(TerminalKind::MalformedFrame),
        }
    }
}

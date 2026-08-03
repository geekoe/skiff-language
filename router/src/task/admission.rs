//! Real `AttemptAdmission` seam (authoritative design "Runtime Admission And
//! Settlement"): after a claim, this seam selects a Runtime that has already
//! admitted the task's frozen execution image and submits one ordinary
//! `request.start` frame carrying the `taskAttempt` header
//! (taskId/attemptId/leaseId). The dispatcher is the ordinary request
//! admission owner (same pool / permit / revalidation / deadline machinery);
//! this seam only proves image admission and maps the dispatcher result to
//! the scheduler's four-decision vocabulary.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
    RuntimeAssemblyTaskAttemptFrameHeader, RuntimeAssemblyTaskInvocationFrameHeader,
    RuntimeAssemblyTaskRequestCallerFrameHeader, RuntimeAssemblyTaskRequestRoutingFrameHeader,
    RuntimeAssemblyTaskRequestStartFrameHeader,
};
use skiff_task_control::model::{DetachedCallTarget, TaskRecord};
use skiff_task_control::scheduler::{AdmissionDecision, AttemptAdmission};

use crate::dispatch::{
    RequestAuthority, RequestDeadline, RequestDispatcher, RoutingEpochSource, TaskAttemptSubmit,
    TaskAttemptSubmitResult,
};
use crate::ws::Clock;

use super::control::DurableTaskControl;
use super::health::TaskControlCounters;

/// Production admission seam.
pub struct RouterTaskAttemptAdmission {
    epoch: Arc<dyn RoutingEpochSource>,
    /// Deferred dispatcher handle (assembled after the scheduler).
    dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
    control: Arc<DurableTaskControl>,
    clock: Arc<dyn Clock>,
    request_timeout_ms: u64,
    counters: Arc<TaskControlCounters>,
    seq: AtomicU64,
}

impl std::fmt::Debug for RouterTaskAttemptAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouterTaskAttemptAdmission")
            .field("request_timeout_ms", &self.request_timeout_ms)
            .finish()
    }
}

impl RouterTaskAttemptAdmission {
    pub fn new(
        epoch: Arc<dyn RoutingEpochSource>,
        dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
        control: Arc<DurableTaskControl>,
        clock: Arc<dyn Clock>,
        request_timeout_ms: u64,
        counters: Arc<TaskControlCounters>,
    ) -> Self {
        Self {
            epoch,
            dispatcher,
            control,
            clock,
            request_timeout_ms,
            counters,
            seq: AtomicU64::new(0),
        }
    }

    /// Captured routing authority for the frozen execution image. `None`
    /// means the image is not admitted by the current epoch (transient: a
    /// runtime may admit it later; D2 has no cold-activation lane).
    fn image_authority(&self, record: &TaskRecord) -> Option<RequestAuthority> {
        let epoch = self.epoch.capture()?;
        let tuple = epoch.registered_tuple();
        if tuple.environment != record.execution.target_environment
            || tuple.assembly != record.execution.assembly
            || tuple.config_snapshot != record.execution.config_snapshot
        {
            return None;
        }
        if !epoch.deployment_projection().contains(&record.execution.deployment) {
            return None;
        }
        Some(RequestAuthority {
            assembly_identity: tuple.assembly.assembly_identity.as_str().to_string(),
            assembly_generation: tuple.generation,
            deployment: record.execution.deployment.clone(),
            session_epoch: crate::session::identity::RuntimeSessionEpoch {
                replica_id: "task-attempt".to_string(),
                connection_generation: 0,
            },
        })
    }

    fn build_request(
        &self,
        record: &TaskRecord,
        authority: &RequestAuthority,
    ) -> Option<RuntimeAssemblyTaskRequestStartFrameHeader> {
        let lease = record.active_lease.as_ref()?;
        let now = self.clock.now_ms();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let request_id = format!(
            "task-attempt:{}-{}-{seq}",
            record.task_id.as_str(),
            lease.attempt_id.as_str()
        );
        let span_id = format!("{:016x}", now.wrapping_add(seq));
        let target = match &record.target {
            DetachedCallTarget::Function { callable } => {
                ("function".to_string(), callable.as_str().to_string())
            }
            DetachedCallTarget::ActorMethod { .. } => return None,
        };
        let deadline = RequestDeadline {
            timeout_ms: self.request_timeout_ms,
            expires_at: super::iso_timestamp(now.saturating_add(self.request_timeout_ms)),
        };
        Some(RuntimeAssemblyTaskRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id,
            mode: "unary".to_string(),
            caller: RuntimeAssemblyTaskRequestCallerFrameHeader {
                kind: "service".to_string(),
            },
            routing: RuntimeAssemblyTaskRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: record.execution.assembly.assembly_identity.clone(),
                assembly_generation: authority.assembly_generation,
                deployment: record.execution.deployment.clone(),
            },
            invocation: RuntimeAssemblyTaskInvocationFrameHeader {
                kind: "task".to_string(),
                target_kind: target.0,
                target: target.1,
            },
            deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
                timeout_ms: deadline.timeout_ms,
                expires_at: deadline.expires_at.clone(),
            }),
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: record.trace.trace_id.clone(),
                span_id,
                parent_span_id: None,
                sampled: None,
            },
            test_effects_enabled: false,
            test_case_capability: None,
            task_attempt: Some(RuntimeAssemblyTaskAttemptFrameHeader {
                task_id: record.task_id.as_str().to_string(),
                attempt_id: lease.attempt_id.as_str().to_string(),
                lease_id: lease.lease_id.as_str().to_string(),
            }),
        })
    }
}

#[async_trait::async_trait]
impl AttemptAdmission for RouterTaskAttemptAdmission {
    async fn admit(&self, record: &TaskRecord) -> AdmissionDecision {
        let Some(authority) = self.image_authority(record) else {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::RejectedProvable {
                reason: "frozen execution image is not admitted by any runtime".to_string(),
            };
        };
        let Some(lease) = record.active_lease.as_ref() else {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: "claimed task record has no active lease".to_string(),
            };
        };
        let Some(header) = self.build_request(record, &authority) else {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: "actor-method task targets are unsupported until stage E".to_string(),
            };
        };
        let Some(dispatcher) = self
            .dispatcher
            .lock()
            .expect("deferred dispatcher lock")
            .clone()
        else {
            // The composition has not finished; the scheduler loop should
            // retry after backoff rather than settle or hot-loop.
            self.counters
                .admissions_uncertain
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::Uncertain {
                reason: "task control plane is not fully assembled".to_string(),
            };
        };
        let request_id = header.request_id.clone();
        let task_id = record.task_id.clone();
        let lease_id = lease.lease_id.clone();
        let attempt_id = lease.attempt_id.clone();
        let deadline_ms = self.clock.now_ms().saturating_add(self.request_timeout_ms);
        let result = dispatcher.task_attempt_submit(TaskAttemptSubmit {
            header,
            payload: record.payload.as_bytes().to_vec(),
            task_id: task_id.as_str().to_string(),
            attempt_id: attempt_id.as_str().to_string(),
            lease_id: lease_id.as_str().to_string(),
        });
        match result {
            TaskAttemptSubmitResult::Accepted { .. } => {
                self.counters
                    .admissions_accepted
                    .fetch_add(1, Ordering::Relaxed);
                self.control
                    .track_attempt(&request_id, &task_id, &lease_id, deadline_ms);
                AdmissionDecision::Accepted
            }
            TaskAttemptSubmitResult::Rejected { reason, .. } => {
                self.counters
                    .admissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                AdmissionDecision::RejectedProvable {
                    reason: format!("runtime admission rejected: {}", reason.as_str()),
                }
            }
        }
    }
}

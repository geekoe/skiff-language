//! Real `AttemptAdmission` seam (authoritative design "Runtime Admission And
//! Settlement"): after a claim, this seam selects a Runtime that has already
//! admitted the task's frozen execution image and submits one ordinary
//! `request.start` frame carrying the `taskAttempt` header
//! (taskId/attemptId/leaseId) for function targets.
//!
//! Actor-method targets (E2b) do not use `request.start`: the seam executes
//! the Actor route layer's **get-or-activate** (authoritative design
//! "Actor-method target") and admits the method as an ordinary Actor
//! invocation through `ActorInvocationRelay` + `actor.owner.invoke`. The five
//! branches are:
//!
//! 1. live incarnation with the same implementation → ordinary admission;
//! 2. no live incarnation but registry entry exists → activate from the
//!    entry's frozen create input (put-if-absent);
//! 3. registry entry lost → restore a minimal entry from the task snapshot
//!    and activate (first successful restore wins);
//! 4. owner runtime reports `ActorUpgradingError` → recoverable release with
//!    the runtime-provided backoff;
//! 5. incarnation / fencing taken over by a different implementation →
//!    `ActorVersionRejectedError` → permanent platform-failed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};
use skiff_runtime_transport::actor_method::{
    ActorLogicalRefFrameHeader, ActorMethodDeadlineFrameHeader, ActorMethodInvokeFrameHeader,
    ACTOR_ARGUMENTS_ENCODING_V1,
};
use skiff_runtime_transport::actor_owner::{
    encode_actor_owner_invoke_frame, ActorOwnerFenceFrameHeader, ActorOwnerInvokeFrameHeader,
    ActorOwnerRouteAuthorityFrameHeader,
};
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::protocol::TelemetryLevel;
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
    RuntimeAssemblyTaskAttemptFrameHeader, RuntimeAssemblyTaskInvocationFrameHeader,
    RuntimeAssemblyTaskRequestCallerFrameHeader, RuntimeAssemblyTaskRequestRoutingFrameHeader,
    RuntimeAssemblyTaskRequestStartFrameHeader,
};
use skiff_task_control::model::{DetachedCallTarget, TaskLease, TaskRecord};
use skiff_task_control::scheduler::{AdmissionDecision, AttemptAdmission};

use crate::actor::{
    pick_owner_candidate, ActivationWaiterOutcome, ActorGetOrCreateRequest, ActorInvokeInput,
    ActorLogicalKey, ActorOwnerRouteAuthority, CatalogQuery, GetOrCreateOutcome, OwnerSettleKind,
    DEFAULT_OWNER_LEASE_TTL_MS,
};
use crate::dispatch::{
    RequestAuthority, RequestDeadline, RequestDispatcher, RoutingEpochSource, TaskAttemptSubmit,
    TaskAttemptSubmitResult,
};
use crate::session::identity::RuntimeSessionEpoch;
use crate::supervisor::actor_sink::ActorFrameSink;
use crate::supervisor::actor::ActorComponents;
use crate::task::{TaskActorOwnerPort, TaskAttemptInvocationCorrelation};
use crate::telemetry::{task_event, TaskTelemetrySink};
use crate::ws::Clock;

use super::actor_target::{snapshot_actor_key, store_declaration_owner_to_frame};
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
    telemetry: Arc<dyn TaskTelemetrySink>,
    seq: AtomicU64,
    /// Actor lane owners consumed by get-or-activate / invocation admission.
    actor: Arc<ActorComponents>,
    /// Owner candidate selection / session resolution / outbound writes.
    actor_port: Arc<dyn TaskActorOwnerPort>,
    /// Bounded actor activation wait budget for get-or-activate branches 2/3.
    activation_deadline_ms: u64,
    /// Deferred actor frame sink (task-attempt invocation terminal
    /// correlation owner; assembled after the control plane).
    actor_sink: Arc<Mutex<Option<Arc<ActorFrameSink>>>>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        epoch: Arc<dyn RoutingEpochSource>,
        dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
        control: Arc<DurableTaskControl>,
        clock: Arc<dyn Clock>,
        request_timeout_ms: u64,
        counters: Arc<TaskControlCounters>,
        telemetry: Arc<dyn TaskTelemetrySink>,
        actor: Arc<ActorComponents>,
        actor_port: Arc<dyn TaskActorOwnerPort>,
        activation_deadline_ms: u64,
        actor_sink: Arc<Mutex<Option<Arc<ActorFrameSink>>>>,
    ) -> Self {
        Self {
            epoch,
            dispatcher,
            control,
            clock,
            request_timeout_ms,
            counters,
            telemetry,
            seq: AtomicU64::new(0),
            actor,
            actor_port,
            activation_deadline_ms,
            actor_sink,
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
            session_epoch: RuntimeSessionEpoch {
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
        let test_case = record.test_case.as_ref();
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
            test_effects_enabled: test_case.is_some(),
            test_case_capability: test_case.map(|authority| {
                authority.test_case_capability.clone()
            }),
            task_attempt: Some(RuntimeAssemblyTaskAttemptFrameHeader {
                task_id: record.task_id.as_str().to_string(),
                attempt_id: lease.attempt_id.as_str().to_string(),
                lease_id: lease.lease_id.as_str().to_string(),
            }),
        })
    }

    fn emit_admission_decision(&self, record: &TaskRecord, decision: &AdmissionDecision) {
        let lease = record.active_lease.as_ref();
        let mut attrs = Map::new();
        if let Some(lease) = lease {
            attrs.insert(
                "attemptId".to_string(),
                Value::String(lease.attempt_id.as_str().to_string()),
            );
            attrs.insert(
                "leaseId".to_string(),
                Value::String(lease.lease_id.as_str().to_string()),
            );
        }
        attrs.insert(
            "targetKind".to_string(),
            Value::String(match record.target {
                DetachedCallTarget::Function { .. } => "function",
                DetachedCallTarget::ActorMethod { .. } => "actorMethod",
            }
            .to_string()),
        );
        match decision {
            AdmissionDecision::Accepted => {
                self.telemetry.emit(task_event(
                    "task.admission.accepted",
                    TelemetryLevel::Info,
                    Some(record.task_id.as_str()),
                    attrs,
                ));
            }
            AdmissionDecision::RejectedProvable { reason } => {
                attrs.insert("reason".to_string(), Value::String(reason.clone()));
                self.telemetry.emit(task_event(
                    "task.admission.rejected",
                    TelemetryLevel::Warn,
                    Some(record.task_id.as_str()),
                    attrs,
                ));
            }
            AdmissionDecision::Uncertain { reason } => {
                attrs.insert("reason".to_string(), Value::String(reason.clone()));
                self.telemetry.emit(task_event(
                    "task.admission.uncertain",
                    TelemetryLevel::Warn,
                    Some(record.task_id.as_str()),
                    attrs,
                ));
            }
            AdmissionDecision::PermanentFailure { reason } => {
                attrs.insert("reason".to_string(), Value::String(reason.clone()));
                self.telemetry.emit(task_event(
                    "task.platform.failed",
                    TelemetryLevel::Error,
                    Some(record.task_id.as_str()),
                    attrs,
                ));
            }
        }
    }

    fn emit_admission_selection(
        &self,
        record: &TaskRecord,
        request_id: Option<&str>,
        runtime: Option<&str>,
        target: Option<&str>,
    ) {
        let mut attrs = Map::new();
        if let Some(lease) = record.active_lease.as_ref() {
            attrs.insert(
                "attemptId".to_string(),
                Value::String(lease.attempt_id.as_str().to_string()),
            );
            attrs.insert(
                "leaseId".to_string(),
                Value::String(lease.lease_id.as_str().to_string()),
            );
        }
        if let Some(runtime) = runtime {
            attrs.insert("runtimeId".to_string(), Value::String(runtime.to_string()));
        }
        if let Some(target) = target {
            attrs.insert("target".to_string(), Value::String(target.to_string()));
        }
        let mut event = task_event(
            "task.admission.selection",
            TelemetryLevel::Info,
            Some(record.task_id.as_str()),
            attrs,
        );
        event.request_id = request_id.map(str::to_string);
        event.trace_id = Some(record.trace.trace_id.clone());
        self.telemetry.emit(event);
    }

    fn emit_artifact_event(&self, record: &TaskRecord, name: &str, reason: &str) {
        let mut attrs = Map::new();
        attrs.insert("reason".to_string(), Value::String(reason.to_string()));
        self.telemetry.emit(task_event(
            name,
            TelemetryLevel::Warn,
            Some(record.task_id.as_str()),
            attrs,
        ));
    }

    /// Actor-method get-or-activate admission (authoritative design
    /// "Actor-method target", five branches).
    async fn admit_actor_method(&self, record: &TaskRecord) -> AdmissionDecision {
        let DetachedCallTarget::ActorMethod {
            actor,
            activation,
            implementation,
            method,
            declaration_owner,
        } = &record.target
        else {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: "actor-method admission received a non-actor target".to_string(),
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
        let key = match snapshot_actor_key(&activation.key) {
            Ok(key) => key,
            Err(reason) => {
                self.counters
                    .admissions_permanent_failure
                    .fetch_add(1, Ordering::Relaxed);
                return AdmissionDecision::PermanentFailure {
                    reason: format!("actor task snapshot key is invalid: {reason}"),
                };
            }
        };
        let test_case = record.test_case.as_ref();
        if test_case.is_some() {
            // F2a: a test-case actor method must belong to the parent service
            // (test-runner-runtime isolation: the capability chain never
            // crosses service). This is checked before the catalog so a
            // cross-service target cannot even consult another service's
            // routing surface.
            if key.service_id != record.owner.as_str() {
                self.counters
                    .admissions_permanent_failure
                    .fetch_add(1, Ordering::Relaxed);
                return AdmissionDecision::PermanentFailure {
                    reason: format!(
                        "test-case actor task target service {} differs from the parent service {}",
                        key.service_id, record.owner.as_str()
                    ),
                };
            }
        }
        let Some(epoch) = self.epoch.capture() else {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::RejectedProvable {
                reason: "no active routing epoch is available".to_string(),
            };
        };
        let Some(authority) = self.image_authority(record) else {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            self.emit_artifact_event(
                record,
                "task.artifact.unavailable",
                "frozen execution image is not admitted by any runtime",
            );
            return AdmissionDecision::RejectedProvable {
                reason: "frozen execution image is not admitted by any runtime".to_string(),
            };
        };
        let query = CatalogQuery::new(
            key.service_id.clone(),
            actor.actor_abi_identity.clone(),
            implementation.clone(),
            method.clone(),
        );
        if !self.actor.catalog_view.has_method(&query) {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: "actor method is absent from the current routing catalog".to_string(),
            };
        }
        let candidates = self.actor_port.candidates(&epoch.registered_tuple());
        let owner = if let Some(authority) = test_case {
            // Execute on the exact origin Runtime connection so the method
            // shares the case's in-memory effect registry; any other owner is
            // a permanent fail-closed condition.
            let origin = RuntimeSessionEpoch {
                replica_id: authority.origin_runtime_id.clone(),
                connection_generation: authority.origin_connection_generation,
            };
            match candidates
                .into_iter()
                .find(|candidate| {
                    candidate.replica_id == origin.replica_id
                        && candidate.connection_generation == origin.connection_generation
                })
            {
                Some(owner) => owner,
                None => {
                    self.counters
                        .admissions_permanent_failure
                        .fetch_add(1, Ordering::Relaxed);
                    return AdmissionDecision::PermanentFailure {
                        reason: format!(
                            "test-case actor task origin Runtime {}#{} is not a current owner candidate",
                            origin.replica_id, origin.connection_generation
                        ),
                    };
                }
            }
        } else {
            let Some(owner) = pick_owner_candidate(&candidates, &key.actor_id_hash).cloned() else {
                self.counters
                    .admissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                return AdmissionDecision::RejectedProvable {
                    reason: "no Runtime is available to own the Actor".to_string(),
                };
            };
            owner
        };
        let owner_connection = format!(
            "{}#{}",
            owner.replica_id, owner.connection_generation
        );
        self.emit_admission_selection(record, None, Some(&owner_connection), Some(method.as_str()));
        let now = self.clock.now_ms();
        let declaration_owner_frame = store_declaration_owner_to_frame(declaration_owner);

        // Branch 1 / 5: a live owner fence decides by implementation identity.
        if let Some(fence) = self.actor.registry.current_owner(&key) {
            if fence.actor_implementation_identity != *implementation {
                return self.version_rejected(record, Some(&fence));
            }
            let fence = self
                .actor
                .registry
                .renew(&key, &fence, DEFAULT_OWNER_LEASE_TTL_MS, now)
                .unwrap_or(fence);
            return self
                .invoke_actor_method(
                    record,
                    lease,
                    &authority,
                    &fence,
                    &key,
                    &declaration_owner_frame,
                    &owner,
                    &owner_connection,
                )
                .await;
        }

        // No live owner: branch 2 (entry create input) or branch 3 (snapshot
        // restore). Both go through the same broker / identity fencing.
        let entry = self.actor.registry.entry(&key);
        let (activation_abi, activation_impl, activation_decl, bootstrap) = match entry {
            Some(entry) => {
                if entry.actor_implementation_identity != *implementation {
                    let fence = self.actor.registry.current_owner(&key);
                    return self.version_rejected(record, fence.as_ref());
                }
                if entry.create_input.is_empty() {
                    // Entry predates create-input freezing: it cannot satisfy
                    // "entry 创建输入", so treat it as an incomplete entry and
                    // restore from the task snapshot (put-if-absent still
                    // keeps identity facts from the first entry).
                    (
                        actor.actor_abi_identity.clone(),
                        implementation.clone(),
                        declaration_owner_frame.clone(),
                        activation.create_input.as_bytes().to_vec(),
                    )
                } else {
                    (
                        entry.actor_abi_identity,
                        entry.actor_implementation_identity,
                        entry.declaration_owner,
                        entry.create_input,
                    )
                }
            }
            None => (
                actor.actor_abi_identity.clone(),
                implementation.clone(),
                declaration_owner_frame.clone(),
                activation.create_input.as_bytes().to_vec(),
            ),
        };
        let deadline_at = now.saturating_add(self.activation_deadline_ms);
        let rpc_id = format!(
            "task-activation:{}-{}",
            record.task_id.as_str(),
            lease.attempt_id.as_str()
        );
        let request = ActorGetOrCreateRequest {
            rpc_id: rpc_id.clone(),
            actor_key: key.clone(),
            actor_abi_identity: activation_abi,
            actor_implementation_identity: activation_impl,
            declaration_owner: activation_decl,
            bootstrap_bytes: bootstrap,
            owner_runtime_id: owner.replica_id.clone(),
            owner_connection: owner_connection.clone(),
            route_authority: ActorOwnerRouteAuthority {
                assembly_identity: authority.assembly_identity.clone(),
                assembly_generation: authority.assembly_generation,
            },
            deadline: Some(ActorMethodDeadlineFrameHeader {
                timeout_ms: self.activation_deadline_ms,
                expires_at: super::iso_timestamp(deadline_at),
            }),
            test_case_capability: test_case.map(|authority| authority.test_case_capability.clone()),
            test_case_parent_request_id: test_case.map(|authority| {
                authority.parent_request_id.clone()
            }),
            now,
        };
        match self.actor.activation_broker.get_or_create(&request) {
            GetOrCreateOutcome::Resolved(_) => self
                .invoke_resolved_actor(
                    record,
                    lease,
                    &authority,
                    &key,
                    implementation,
                    &declaration_owner_frame,
                    &owner,
                    &owner_connection,
                )
                .await,
            GetOrCreateOutcome::StartedActivation { .. } | GetOrCreateOutcome::Joined => {
                match self
                    .wait_for_activation(&rpc_id, deadline_at.saturating_add(1_000))
                    .await
                {
                    Some(ActivationWaiterOutcome::Resolved { .. }) => self
                        .invoke_resolved_actor(
                            record,
                            lease,
                            &authority,
                            &key,
                            implementation,
                            &declaration_owner_frame,
                            &owner,
                            &owner_connection,
                        )
                        .await,
                    Some(ActivationWaiterOutcome::Failed { code }) => {
                        self.activation_failure_decision(&code)
                    }
                    None => {
                        self.counters
                            .admissions_rejected
                            .fetch_add(1, Ordering::Relaxed);
                        AdmissionDecision::RejectedProvable {
                            reason: "actor activation deadline elapsed without an ACK".to_string(),
                        }
                    }
                }
            }
            GetOrCreateOutcome::Saturated => {
                self.counters
                    .admissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                AdmissionDecision::RejectedProvable {
                    reason: "actor activation claim budget reached".to_string(),
                }
            }
            GetOrCreateOutcome::LineageConflict => {
                self.counters
                    .admissions_rejected
                    .fetch_add(1, Ordering::Relaxed);
                AdmissionDecision::RejectedProvable {
                    reason: "actor activation lineage conflict".to_string(),
                }
            }
            GetOrCreateOutcome::Failed { code } => self.activation_failure_decision(&code),
        }
    }

    async fn invoke_resolved_actor(
        &self,
        record: &TaskRecord,
        lease: &TaskLease,
        authority: &RequestAuthority,
        key: &ActorLogicalKey,
        implementation: &skiff_artifact_model::ActorImplementationIdentity,
        declaration_owner: &skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader,
        owner: &RuntimeSessionEpoch,
        owner_connection: &str,
    ) -> AdmissionDecision {
        let Some(fence) = self.actor.registry.current_owner(key) else {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::RejectedProvable {
                reason: "actor activation resolved without a committed owner fence"
                    .to_string(),
            };
        };
        if fence.actor_implementation_identity != *implementation {
            return self.version_rejected(record, Some(&fence));
        }
        self.invoke_actor_method(
            record,
            lease,
            authority,
            &fence,
            key,
            declaration_owner,
            owner,
            owner_connection,
        )
        .await
    }

    /// Bounded async wait for one get-or-create waiter outcome (the broker
    /// remains a synchronous reducer; outcome inserts wake this waiter).
    async fn wait_for_activation(
        &self,
        rpc_id: &str,
        deadline_at: u64,
    ) -> Option<ActivationWaiterOutcome> {
        let notify = self.actor.activation_broker.notifier();
        loop {
            let notified = notify.notified();
            if let Some(outcome) = self.actor.activation_broker.outcome_for(rpc_id) {
                return Some(parse_waiter_outcome(&outcome));
            }
            let now = self.clock.now_ms();
            if now >= deadline_at {
                return None;
            }
            let remaining = (deadline_at - now).min(50);
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(Duration::from_millis(remaining)) => {}
            }
        }
    }

    fn activation_failure_decision(&self, code: &str) -> AdmissionDecision {
        if code == "IncarnationReplaced" {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: format!(
                    "ActorVersionRejectedError: actor incarnation was replaced ({code})"
                ),
            };
        }
        self.counters
            .admissions_rejected
            .fetch_add(1, Ordering::Relaxed);
        AdmissionDecision::RejectedProvable {
            reason: format!("actor activation failed closed: {code}"),
        }
    }

    fn version_rejected(
        &self,
        record: &TaskRecord,
        fence: Option<&crate::actor::ActorOwnerFence>,
    ) -> AdmissionDecision {
        self.counters
            .admissions_permanent_failure
            .fetch_add(1, Ordering::Relaxed);
        let accepted = fence
            .map(|fence| fence.actor_implementation_identity.as_str())
            .unwrap_or("unknown");
        let DetachedCallTarget::ActorMethod { implementation, .. } = &record.target else {
            return AdmissionDecision::PermanentFailure {
                reason: "actor version rejection on non-actor target".to_string(),
            };
        };
        AdmissionDecision::PermanentFailure {
            reason: format!(
                "ActorVersionRejectedError: task implementation {} was superseded by {accepted}",
                implementation.as_str()
            ),
        }
    }

    /// Admit one actor method invocation on an existing owner fence (branches
    /// 1/2/3 resolution): relay + register task-attempt correlation + write
    /// `actor.owner.invoke` with the frozen task payload.
    async fn invoke_actor_method(
        &self,
        record: &TaskRecord,
        lease: &TaskLease,
        authority: &RequestAuthority,
        fence: &crate::actor::ActorOwnerFence,
        key: &ActorLogicalKey,
        declaration_owner: &skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader,
        owner: &RuntimeSessionEpoch,
        owner_connection: &str,
    ) -> AdmissionDecision {
        let DetachedCallTarget::ActorMethod {
            actor, implementation, method, ..
        } = &record.target
        else {
            self.counters
                .admissions_permanent_failure
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::PermanentFailure {
                reason: "actor invoke received a non-actor target".to_string(),
            };
        };
        let now = self.clock.now_ms();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let request_id = format!(
            "task-attempt:{}-{}-{seq}",
            record.task_id.as_str(),
            lease.attempt_id.as_str()
        );
        let invocation_id = format!("{request_id}:invoke");
        let expires_at = super::iso_timestamp(now.saturating_add(self.request_timeout_ms));
        let cancellation_correlation = format!("{invocation_id}:cancel");
        let test_case = record.test_case.as_ref();
        let invoke = ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.invoke".to_string(),
            invocation_id: invocation_id.clone(),
            actor_ref: ActorLogicalRefFrameHeader {
                service_id: key.service_id.clone(),
                actor_type_identity: key.actor_type_identity.clone(),
                actor_id_type_identity: key.actor_id_type_identity.clone(),
                actor_id_encoding_version: key.actor_id_encoding_version.clone(),
                canonical_actor_id_key_bytes_base64: key
                    .canonical_actor_id_key_bytes_base64
                    .clone(),
                actor_id_hash: key.actor_id_hash.clone(),
                epoch: fence.epoch,
            },
            declaration_owner: declaration_owner.clone(),
            actor_abi_identity: actor.actor_abi_identity.clone(),
            actor_implementation_identity: implementation.clone(),
            method_identity: method.clone(),
            arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.to_string(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: self.request_timeout_ms,
                expires_at: expires_at.clone(),
            },
            cancellation_correlation: cancellation_correlation.clone(),
            trace_id: Some(record.trace.trace_id.clone()),
            test_case_capability: test_case
                .map(|authority| authority.test_case_capability.clone()),
            test_case_parent_request_id: test_case.map(|authority| {
                authority.parent_request_id.clone()
            }),
        };
        let owner_invoke = ActorOwnerInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.owner.invoke".to_string(),
            target_runtime_id: fence.owner_runtime_id.clone(),
            owner_fence: ActorOwnerFenceFrameHeader {
                owner_runtime_id: fence.owner_runtime_id.clone(),
                owner_lease_id: fence.owner_lease_id.clone(),
                epoch: fence.epoch,
                actor_abi_identity: fence.actor_abi_identity.clone(),
                actor_implementation_identity: fence.actor_implementation_identity.clone(),
                declaration_owner: fence.declaration_owner.clone(),
            },
            invoke,
            route_authority: ActorOwnerRouteAuthorityFrameHeader {
                assembly_identity: authority.assembly_identity.clone(),
                assembly_generation: authority.assembly_generation,
            },
            activation_bootstrap: None,
        };
        let bytes = match encode_actor_owner_invoke_frame(&owner_invoke, record.payload.as_bytes())
        {
            Ok(bytes) => bytes,
            Err(error) => {
                self.counters
                    .admissions_permanent_failure
                    .fetch_add(1, Ordering::Relaxed);
                return AdmissionDecision::PermanentFailure {
                    reason: format!("actor owner invoke encode failed: {error}"),
                };
            }
        };
        let route_authority = ActorOwnerRouteAuthority {
            assembly_identity: authority.assembly_identity.clone(),
            assembly_generation: authority.assembly_generation,
        };
        self.actor.lease_scheduler.mark_live(key, now, owner_connection);
        let input = ActorInvokeInput {
            invocation_id: invocation_id.clone(),
            caller_connection: format!("task-attempt:{request_id}"),
            caller_runtime_id: "task-control".to_string(),
            owner_fence: fence.clone(),
            owner_connection: owner_connection.to_string(),
            route_authority: route_authority.clone(),
            correlation: cancellation_correlation,
            deadline: Some(ActorMethodDeadlineFrameHeader {
                timeout_ms: self.request_timeout_ms,
                expires_at,
            }),
            test_case_capability: test_case
                .map(|authority| authority.test_case_capability.clone()),
            now,
        };
        if let Err(error) = self.actor.relay.invoke(&input) {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::RejectedProvable {
                reason: format!("actor invocation relay rejected the attempt: {error}"),
            };
        }
        let Some(actor_sink) = self
            .actor_sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            let _ = self.actor.relay.on_owner_settle(
                &invocation_id,
                fence,
                owner_connection,
                OwnerSettleKind::Error,
            );
            self.counters
                .admissions_uncertain
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::Uncertain {
                reason: "actor frame sink is not assembled".to_string(),
            };
        };
        let task_attempt = TaskAttemptInvocationCorrelation {
            request_id: request_id.clone(),
            task_id: record.task_id.as_str().to_string(),
            attempt_id: lease.attempt_id.as_str().to_string(),
            lease_id: lease.lease_id.as_str().to_string(),
        };
        if let Err(error) =
            actor_sink.register_task_attempt_invocation(&invocation_id, task_attempt, owner.clone(), fence.clone())
        {
            let _ = self.actor.relay.on_owner_settle(
                &invocation_id,
                fence,
                owner_connection,
                OwnerSettleKind::Error,
            );
            self.counters
                .admissions_uncertain
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::Uncertain {
                reason: format!("actor task-attempt correlation failed: {error}"),
            };
        }
        let Some(owner_session) = self.actor_port.current_session_by_replica(&owner.replica_id)
        else {
            actor_sink.unregister_invocation(&invocation_id);
            let _ = self.actor.relay.on_owner_settle(
                &invocation_id,
                fence,
                owner_connection,
                OwnerSettleKind::Error,
            );
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::RejectedProvable {
                reason: format!("owner runtime {} has no registered session", owner.replica_id),
            };
        };
        if let Err(error) = self.actor_port.write(&owner_session, bytes) {
            actor_sink.unregister_invocation(&invocation_id);
            let _ = self.actor.relay.on_owner_settle(
                &invocation_id,
                fence,
                owner_connection,
                OwnerSettleKind::Error,
            );
            self.counters
                .admissions_uncertain
                .fetch_add(1, Ordering::Relaxed);
            return AdmissionDecision::Uncertain {
                reason: format!("actor owner invoke write failed: {error}"),
            };
        }
        self.control.track_attempt(
            &request_id,
            &record.task_id,
            &lease.lease_id,
            now.saturating_add(self.request_timeout_ms),
        );
        self.counters
            .admissions_accepted
            .fetch_add(1, Ordering::Relaxed);
        AdmissionDecision::Accepted
    }
}

fn parse_waiter_outcome(outcome: &str) -> ActivationWaiterOutcome {
    if let Some(epoch) = outcome.strip_prefix("resolved:") {
        if let Ok(epoch) = epoch.parse::<u64>() {
            return ActivationWaiterOutcome::Resolved { epoch };
        }
    }
    if let Some(code) = outcome.strip_prefix("failed:") {
        return ActivationWaiterOutcome::Failed {
            code: code.to_string(),
        };
    }
    ActivationWaiterOutcome::Failed {
        code: outcome.to_string(),
    }
}

#[async_trait::async_trait]
impl AttemptAdmission for RouterTaskAttemptAdmission {
    async fn admit(&self, record: &TaskRecord) -> AdmissionDecision {
        let decision = if matches!(record.target, DetachedCallTarget::ActorMethod { .. }) {
            self.admit_actor_method(record).await
        } else {
            self.admit_function(record).await
        };
        self.emit_admission_decision(record, &decision);
        if record.test_case.is_some() {
            // F2a test-case submissions gate their response on the first
            // attempt admission; publish the outcome only for those tasks so
            // the production fast path is untouched.
            self.control.report_first_admission(&record.task_id, &decision);
        }
        decision
    }
}

impl RouterTaskAttemptAdmission {
    async fn admit_function(&self, record: &TaskRecord) -> AdmissionDecision {
        let Some(authority) = self.image_authority(record) else {
            self.counters
                .admissions_rejected
                .fetch_add(1, Ordering::Relaxed);
            self.emit_artifact_event(
                record,
                "task.artifact.unavailable",
                "frozen execution image is not admitted by any runtime",
            );
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
                reason: "task target cannot be formed into a runtime request".to_string(),
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
        let prefer_session = record.test_case.as_ref().map(|authority| RuntimeSessionEpoch {
            replica_id: authority.origin_runtime_id.clone(),
            connection_generation: authority.origin_connection_generation,
        });
        let result = dispatcher.task_attempt_submit(TaskAttemptSubmit {
            header,
            payload: record.payload.as_bytes().to_vec(),
            task_id: task_id.as_str().to_string(),
            attempt_id: attempt_id.as_str().to_string(),
            lease_id: lease_id.as_str().to_string(),
            prefer_session,
        });
        match result {
            TaskAttemptSubmitResult::Accepted {
                session_epoch, ..
            } => {
                self.counters
                    .admissions_accepted
                    .fetch_add(1, Ordering::Relaxed);
                let target = match &record.target {
                    DetachedCallTarget::Function { callable } => callable.as_str(),
                    DetachedCallTarget::ActorMethod { .. } => "actorMethod",
                };
                self.emit_admission_selection(
                    record,
                    Some(&request_id),
                    Some(&session_epoch.replica_id),
                    Some(target),
                );
                self.control
                    .track_attempt(&request_id, &task_id, &lease_id, deadline_ms);
                AdmissionDecision::Accepted
            }
            TaskAttemptSubmitResult::Rejected { reason, .. } => {
                if record.test_case.is_some()
                    && reason == crate::dispatch::SubmitRejectReason::NoCandidate
                {
                    // The test case's exact origin Runtime connection is not
                    // a current candidate; any other placement would cross
                    // the case's connection boundary, so this is permanent.
                    self.counters
                        .admissions_permanent_failure
                        .fetch_add(1, Ordering::Relaxed);
                    return AdmissionDecision::PermanentFailure {
                        reason: "test-case task origin Runtime connection is unavailable"
                            .to_string(),
                    };
                }
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

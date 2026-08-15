//! X6 task request seam.
//!
//! Durable function tasks are fresh requests, not VM child invocations. This
//! module owns the request-side marker, exact task submit routing, and the
//! fail-closed payload/timing gate. The exact F6 tuple/record plan is not yet
//! present in the linked signature, and K6 has not yet surfaced the resolved
//! timing value, so the seam refuses to guess parameter names or timing
//! semantics.

use std::{future::Future, pin::Pin, sync::Arc};

use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_capability_context::{
    ActivationIdentityControl, TaskCallerKind, TaskSubmitControlMessage, TaskSubmitControlRequest,
    TaskSubmitResponseControl, TaskSubmitTimingControl,
};
use skiff_runtime_linked_bytecode::{LinkedTaskTarget, LinkedTaskTiming};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{vm_heap::VmHeap, vm_value::ValueSlot};
use skiff_runtime_scheduler::{BytecodeChildHandoff, BytecodePortFailure, BytecodeSchedulerError};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, TaskDispatchIndex, VmBudget, VmFiber, VmResumeToken,
};

use crate::{RequestEnvelope, RequestError, RequestResult};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeTaskSubmitError {
    #[error("task submit port is absent: {0}")]
    PortUnavailable(String),
    #[error("task submit rejected ({code}): {message}")]
    Rejected { code: String, message: String },
    #[error("task submit response protocol error: {0}")]
    Protocol(String),
    #[error("task submit writer channel closed")]
    Closed,
}

/// Request-owned outbound task submit port.
///
/// The host installs this with the same `RouterWriterMessage` channel used by
/// the existing actor/task control plane. The request mux never constructs a
/// child heap or child owner for a task: submission is a fresh durable request
/// whose parent continues only after durable acceptance.
pub trait BytecodeTaskSubmitter: Send + Sync + 'static {
    fn submit<'a>(
        &'a self,
        message: TaskSubmitControlMessage,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TaskSubmitResponseControl, BytecodeTaskSubmitError>>
                + Send
                + 'a,
        >,
    >;
}

/// Fail-closed submitter used until the host can install the outbound writer.
#[derive(Default)]
pub struct FailClosedTaskSubmitter;

impl BytecodeTaskSubmitter for FailClosedTaskSubmitter {
    fn submit<'a>(
        &'a self,
        _message: TaskSubmitControlMessage,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TaskSubmitResponseControl, BytecodeTaskSubmitError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            Err(BytecodeTaskSubmitError::PortUnavailable(
                task_required_fact().to_string(),
            ))
        })
    }
}

/// Exact facts the request mux needs for one task child submission.
#[derive(Clone)]
pub struct BytecodeTaskChildComposition {
    pub submitter: Arc<dyn BytecodeTaskSubmitter>,
    pub caller_request_id: String,
    pub runtime_id: String,
    pub activation_identity: Option<ActivationIdentityControl>,
}

impl Default for BytecodeTaskChildComposition {
    fn default() -> Self {
        Self {
            submitter: Arc::new(FailClosedTaskSubmitter),
            caller_request_id: String::new(),
            runtime_id: String::new(),
            activation_identity: None,
        }
    }
}

impl BytecodeTaskChildComposition {
    pub fn is_available(&self) -> bool {
        !self.caller_request_id.is_empty()
            && !self.runtime_id.is_empty()
            && self.activation_identity.is_some()
    }
}

pub(crate) fn is_task_request(request: &RequestEnvelope) -> bool {
    request
        .extra
        .get("task")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn task_arguments(
    _request: &RequestEnvelope,
    _entry: &DeploymentExecutionEntry,
    _heap: &mut dyn VmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    Err(RequestError::Unsupported(task_required_fact().to_string()))
}

pub(crate) fn task_required_fact() -> &'static str {
    "durable task recoverable payload requires the exact F6 linked parameter \
     tuple/record plan and a K6-resolved dispatch timing value; the current \
     linked signature does not retain parameter names or resolved timing"
}

pub(crate) fn task_timing_control(
    target: &LinkedTaskTarget,
) -> Result<TaskSubmitTimingControl, BytecodeTaskSubmitError> {
    match target.timing() {
        LinkedTaskTiming::Immediate => Ok(TaskSubmitTimingControl::Immediate),
        LinkedTaskTiming::After { expression } | LinkedTaskTiming::At { expression } => {
            Err(BytecodeTaskSubmitError::PortUnavailable(format!(
                "task timing expression {expression} has not been resolved by K6; \
                 exact task timing must be supplied by the VM dispatch request"
            )))
        }
    }
}

pub(crate) fn task_target_by_dispatch_index<'a>(
    image: &'a DeploymentExecutionImage,
    index: TaskDispatchIndex,
) -> Option<&'a LinkedTaskTarget> {
    image
        .task_targets()
        .find(|target| target.index().get() == index.get())
}

pub(crate) fn execute_task_child(
    invocation: ChildInvocation,
    _heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    composition: &BytecodeTaskChildComposition,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Task(index) = invocation.target() else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };
    let image = invocation.resume().image();
    let reason = match task_target_by_dispatch_index(image, index) {
        None => "task dispatch table row is absent".to_string(),
        Some(target) if !composition.is_available() => {
            "task child requires exact host activation identity, caller request id and runtime id; the current bytecode request has none"
                .to_string()
        }
        Some(target) => match task_timing_control(target) {
            Err(error) => error.to_string(),
            Ok(_) => task_required_fact().to_string(),
        },
    };
    Err(BytecodePortFailure::input(
        BytecodeSchedulerError::Port(reason),
        invocation,
    ))
}

// K6's task dispatch handoff is not merged yet; this is the exact fresh-request
// composition seam that consumes the K6 payload and timing facts.
#[allow(dead_code)]
pub(crate) fn task_submit_message_from_composition(
    deployment: &ServiceDeploymentRef,
    protocol_identity: &str,
    target: &LinkedTaskTarget,
    payload: &[u8],
    rpc_id: &str,
    composition: &BytecodeTaskChildComposition,
) -> Result<TaskSubmitControlMessage, BytecodeTaskSubmitError> {
    if rpc_id.trim().is_empty() {
        return Err(BytecodeTaskSubmitError::PortUnavailable(
            "task child rpc id must not be empty".to_string(),
        ));
    }
    let caller_request_id = composition.caller_request_id.trim().to_string();
    if caller_request_id.is_empty() {
        return Err(BytecodeTaskSubmitError::PortUnavailable(
            "task child requires the current caller request id".to_string(),
        ));
    }
    let runtime_id = composition.runtime_id.trim().to_string();
    if runtime_id.is_empty() {
        return Err(BytecodeTaskSubmitError::PortUnavailable(
            "task child requires the current runtime id".to_string(),
        ));
    }
    let activation_identity = composition.activation_identity.clone().ok_or_else(|| {
        BytecodeTaskSubmitError::PortUnavailable(
            "task child requires the exact activation identity from the current request"
                .to_string(),
        )
    })?;
    let timing = task_timing_control(target)?;
    let target_identity = target.target_identity().trim();
    if target_identity.is_empty() {
        return Err(BytecodeTaskSubmitError::PortUnavailable(
            "task target identity must not be empty".to_string(),
        ));
    }
    Ok(TaskSubmitControlMessage {
        request: TaskSubmitControlRequest {
            rpc_id: rpc_id.to_string(),
            runtime_id,
            target_kind: "function".to_string(),
            service_id: deployment.service_id.clone(),
            service_version: deployment.contract_version.clone(),
            service_protocol_identity: protocol_identity.to_string(),
            target: target_identity.to_string(),
            task_id: None,
            build_id: Some(deployment.deployment_artifact_identity.to_string()),
            activation_identity,
            caller_request_id: Some(caller_request_id),
            timing,
            trace_id: None,
            caller_target: Some(target_identity.to_string()),
            max_queue_wait_ms: None,
            actor_method: None,
        },
        payload: payload.to_vec(),
        caller_kind: TaskCallerKind::Request,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{
        CallableEffectSummary, DeploymentArtifactIdentity, DeploymentRevision, ParamModeIr,
        ServiceDeploymentRef,
    };
    use skiff_runtime_linked_bytecode::{
        FunctionIndex, LinkedCallableSignature, LinkedValueDropPlan, LinkedValueTransferPlan,
        TaskTargetIndex, TypeIndex,
    };

    use super::*;

    fn deployment() -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: "test.skiff/task".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision:task"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("build:task"),
        }
    }

    fn snapshot_plan() -> LinkedValueTransferPlan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    }

    fn signature() -> LinkedCallableSignature {
        LinkedCallableSignature::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([ParamModeIr::Value]),
            Box::new([snapshot_plan()]),
            Box::new([]),
            Box::new([]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("test task signature is canonical")
    }

    fn target(timing: LinkedTaskTiming) -> LinkedTaskTarget {
        LinkedTaskTarget::new(
            TaskTargetIndex::new(1),
            "test.skiff/task:work",
            FunctionIndex::new(1),
            signature(),
            timing,
        )
        .expect("test task target is canonical")
    }

    fn activation() -> ActivationIdentityControl {
        ActivationIdentityControl {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            generation: 1,
            runtime_replica_id: "runtime-test".to_string(),
            deployment_revision: DeploymentRevision::new("revision:task"),
        }
    }

    fn composition() -> BytecodeTaskChildComposition {
        BytecodeTaskChildComposition {
            submitter: Arc::new(FailClosedTaskSubmitter),
            caller_request_id: "caller-request".to_string(),
            runtime_id: "runtime-test".to_string(),
            activation_identity: Some(activation()),
        }
    }

    fn task_envelope() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "task-request".to_string(),
            mode: "unary".to_string(),
            target: "function:main.work".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: Some("test.skiff/task".to_string()),
            build_id: "build:task".to_string(),
            service_protocol_identity: "protocol:task".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: None,
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
            payload_bytes: vec![1, 2, 3],
            extra: serde_json::Map::from_iter([("task".to_string(), json!(true))]),
        }
    }

    #[test]
    fn task_marker_is_exact_and_not_inferred_from_http_payload() {
        assert!(is_task_request(&task_envelope()));
        let mut envelope = task_envelope();
        envelope.extra.remove("task");
        assert!(!is_task_request(&envelope));
    }

    #[test]
    fn task_payload_required_fact_names_f6_owner() {
        assert!(task_required_fact().contains("F6 linked parameter"));
    }

    #[test]
    fn task_submit_message_preserves_exact_target_payload_timing_and_caller() {
        let message = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::Immediate),
            b"exact-payload",
            "rpc:task-child",
            &composition(),
        )
        .expect("exact task facts must compose a fresh submit message");

        assert_eq!(message.request.target_kind, "function");
        assert_eq!(message.request.service_id, "test.skiff/task");
        assert_eq!(message.request.service_version, "1.0.0");
        assert_eq!(message.request.service_protocol_identity, "protocol:task");
        assert_eq!(message.request.target, "test.skiff/task:work");
        assert_eq!(message.request.build_id.as_deref(), Some("build:task"));
        assert_eq!(
            message.request.caller_request_id.as_deref(),
            Some("caller-request")
        );
        assert_eq!(
            message.request.caller_target.as_deref(),
            Some("test.skiff/task:work")
        );
        assert_eq!(message.request.timing, TaskSubmitTimingControl::Immediate);
        assert_eq!(message.payload, b"exact-payload");
        assert_eq!(message.caller_kind, TaskCallerKind::Request);
        assert!(message.request.actor_method.is_none());
    }

    #[test]
    fn task_submit_message_rejects_unresolved_after_timing() {
        let error = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::After { expression: 4 }),
            b"payload",
            "rpc:task-child",
            &composition(),
        )
        .expect_err("unresolved timing must fail before submission");
        assert!(error.to_string().contains("resolved by K6"));
    }

    #[test]
    fn task_submit_message_fails_closed_without_activation_identity() {
        let mut composition = composition();
        composition.activation_identity = None;
        let error = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::Immediate),
            b"payload",
            "rpc:task-child",
            &composition,
        )
        .expect_err("missing activation identity must fail closed");
        assert!(error.to_string().contains("activation identity"));
    }

    #[test]
    fn default_task_child_composition_fails_closed() {
        let composition = BytecodeTaskChildComposition::default();
        assert!(!composition.is_available());
        assert!(
            composition.caller_request_id.is_empty()
                && composition.runtime_id.is_empty()
                && composition.activation_identity.is_none()
        );
    }
}

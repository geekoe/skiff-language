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
    let target_index = index.task_target_index()?;
    image
        .task_targets()
        .find(|target| target.index() == target_index)
}

pub(crate) fn task_child_failure_reason(
    image: &DeploymentExecutionImage,
    index: TaskDispatchIndex,
    composition: &BytecodeTaskChildComposition,
) -> String {
    match task_target_by_dispatch_index(image, index) {
        None => "task dispatch table row is absent".to_string(),
        Some(_target) if !composition.is_available() => {
            "task child requires exact host activation identity, caller request id and runtime id; the current bytecode request has none"
                .to_string()
        }
        Some(target) => match task_timing_control(target) {
            Err(error) => error.to_string(),
            Ok(_) => task_required_fact().to_string(),
        },
    }
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
    let reason = task_child_failure_reason(image, index, composition);
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
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::OnceLock,
    };

    use serde_json::json;
    use skiff_artifact_model::{
        CallableEffectSummary, DeploymentArtifactIdentity, DeploymentRevision, ParamModeIr,
        ServiceDeploymentRef,
    };
    use skiff_compiler::{
        authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
        CompilerPlatformSources,
    };
    use skiff_runtime_linked_bytecode::{
        FunctionIndex, LinkedCallableSignature, LinkedValueDropPlan, LinkedValueTransferPlan,
        TaskTargetIndex, TypeIndex,
    };
    use skiff_runtime_linker::{
        link_deployment_execution_image, DeploymentExecutionImage, LinkLimits,
    };
    use skiff_runtime_loader::{
        DeploymentBytecodeLoader, FilesystemDeploymentBytecodeContentResolver,
    };

    use super::*;

    fn task_image() -> std::sync::Arc<DeploymentExecutionImage> {
        static IMAGE: OnceLock<std::sync::Arc<DeploymentExecutionImage>> = OnceLock::new();
        std::sync::Arc::clone(IMAGE.get_or_init(|| {
            let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("request crate must be under the repository root")
                .to_path_buf();
            let fixture_root = repository_root
                .join("runtime/host/tests/fixtures/bytecode-vm-phase-6/task-positive");
            let artifact_root = std::env::temp_dir().join(format!(
                "skiff-request-task-child-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock must be after the Unix epoch")
                    .as_nanos()
            ));
            fs::create_dir_all(&artifact_root).unwrap();
            let sources = CompilerPlatformSources::new(&repository_root)
                .expect("open repository platform sources");
            seed_official_std_package(&sources, &artifact_root)
                .expect("seed canonical std into task fixture store");
            let receipt = build_authoring_object(
                &sources,
                AuthoringObject::Package,
                &fixture_root,
                &artifact_root,
                "skiff-test",
                true,
            )
            .expect("task fixture publishes through production authoring");
            let deployment = serde_json::from_value(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("task authoring receipt carries deployment"),
            )
            .expect("task deployment receipt remains typed");
            let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
                .expect("open task fixture store");
            let hydrated = DeploymentBytecodeLoader::new(&resolver)
                .load(&deployment)
                .expect("load task fixture closure");
            let image = std::sync::Arc::new(
                link_deployment_execution_image(hydrated, &task_link_limits())
                    .expect("link task fixture image"),
            );
            fs::remove_dir_all(&artifact_root).unwrap();
            image
        }))
    }

    fn task_link_limits() -> LinkLimits {
        LinkLimits {
            max_packages: u64::MAX,
            max_root_specializations: u64::MAX,
            max_specializations: u64::MAX,
            max_code_words_per_function: u64::MAX,
            max_total_code_words: u64::MAX,
            max_relocations_per_function: u64::MAX,
            max_total_relocations: u64::MAX,
            max_image_table_entries: u64::MAX,
            max_total_image_table_entries: u64::MAX,
            max_total_function_table_entries: u64::MAX,
            max_type_nesting_depth: u64::MAX,
            max_expanded_type_nodes: u64::MAX,
            max_expanded_type_bytes: u64::MAX,
            max_constant_graph_nodes: u64::MAX,
            max_constant_graph_edges: u64::MAX,
        }
    }

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
    fn task_dispatch_index_lookup_resolves_exact_linked_row() {
        let image = task_image();
        let target = image
            .task_targets()
            .next()
            .expect("task fixture links a task dispatch row")
            .clone();
        let dispatch = TaskDispatchIndex::from_task_target_index(target.index())
            .expect("linked task target maps to a dispatch index");

        let found = task_target_by_dispatch_index(&image, dispatch)
            .expect("linked task dispatch row must be found");
        assert_eq!(found, &target);
    }

    #[test]
    fn task_dispatch_index_missing_and_out_of_range_fail_closed() {
        let image = task_image();
        let composition = BytecodeTaskChildComposition::default();

        assert!(
            TaskDispatchIndex::try_new(0).is_none(),
            "the reserved zero dispatch index is not constructible"
        );
        let out_of_range = TaskDispatchIndex::try_new(u32::MAX).expect("max is representable");
        let reason = task_child_failure_reason(&image, out_of_range, &composition);
        assert!(
            reason.contains("task dispatch table row is absent"),
            "an out-of-range dispatch index must fail closed: {reason}"
        );
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
    fn task_submit_message_uses_linked_row_target_with_exact_payload() {
        let image = task_image();
        let target = image
            .task_targets()
            .next()
            .expect("task fixture links a task dispatch row")
            .clone();

        let message = task_submit_message_from_composition(
            image.owner().deployment(),
            image.service_protocol_identity().as_str(),
            &target,
            b"exact-payload",
            "rpc:task-child",
            &composition(),
        )
        .expect("exact linked task facts must compose a fresh submit message");

        assert_eq!(message.request.target, target.target_identity());
        assert_eq!(
            message.request.service_id,
            image.owner().deployment().service_id
        );
        assert_eq!(
            message.request.service_version,
            image.owner().deployment().contract_version
        );
        assert_eq!(
            message.request.service_protocol_identity,
            image.service_protocol_identity().as_str()
        );
        assert_eq!(message.payload, b"exact-payload");
        assert_eq!(message.request.timing, TaskSubmitTimingControl::Immediate);
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

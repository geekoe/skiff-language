//! X6 task request seam.
//!
//! Durable function tasks are fresh requests, not VM child invocations. This
//! module owns the request-side marker, exact task submit routing, and the
//! fail-closed payload/timing gate. The seam consumes the exact F6 tuple/record
//! plan and parameter names, and refuses to guess missing K6 timing values.

use std::{future::Future, pin::Pin, sync::Arc};

use skiff_artifact_model::{LiteralIr, ServiceDeploymentRef, TypeRefIr};
use skiff_runtime_boundary::{
    binary::encode_recoverable_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind, PayloadServiceRef},
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, TaskCallerKind, TaskSubmitControlMessage, TaskSubmitControlRequest,
    TaskSubmitResponseControl, TaskSubmitTimingControl,
};
use skiff_runtime_linked_bytecode::{
    LinkedShapeEntry, LinkedTaskPayloadPlan, LinkedTaskTarget, LinkedTaskTiming, LinkedTypeEntry,
    TypeIndex,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    runtime_value::{RuntimeObject, RuntimeObjectFields, RuntimeValue},
    type_plan::builtins::artifact_type_ref_label,
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
    vm_heap::VmHeap,
    vm_value::{ValueKind, ValueSlot},
};
use skiff_runtime_vm::TaskDispatchIndex;

use crate::{vm_heap::RequestVmHeap, RequestEnvelope, RequestError, RequestResult};

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
    fn submit(
        &self,
        message: TaskSubmitControlMessage,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TaskSubmitResponseControl, BytecodeTaskSubmitError>>
                + Send
                + 'static,
        >,
    >;
}

/// Fail-closed submitter used until the host can install the outbound writer.
#[derive(Default)]
pub struct FailClosedTaskSubmitter;

impl BytecodeTaskSubmitter for FailClosedTaskSubmitter {
    fn submit(
        &self,
        _message: TaskSubmitControlMessage,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TaskSubmitResponseControl, BytecodeTaskSubmitError>>
                + Send
                + 'static,
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
     tuple/record plan, its retained parameter names, and a K6-resolved \
     dispatch timing value before submission"
}

fn task_error(message: impl Into<String>) -> BytecodeTaskSubmitError {
    BytecodeTaskSubmitError::PortUnavailable(message.into())
}

fn checked_linked_type_entry<'a>(
    image: &'a DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<&'a LinkedTypeEntry, BytecodeTaskSubmitError> {
    let position = usize::try_from(type_index.get()).map_err(|_| {
        task_error(format!(
            "linked task type {} overflows usize",
            type_index.get()
        ))
    })?;
    image
        .types()
        .get(position)
        .filter(|entry| entry.index() == type_index)
        .ok_or_else(|| {
            task_error(format!(
                "linked task type row {} is absent",
                type_index.get()
            ))
        })
}

fn unique_type_index_for_ref(
    image: &DeploymentExecutionImage,
    ty: &TypeRefIr,
) -> Result<TypeIndex, BytecodeTaskSubmitError> {
    let mut matches = image
        .types()
        .iter()
        .filter(|entry| entry.type_ref() == ty)
        .map(LinkedTypeEntry::index);
    let first = matches
        .next()
        .ok_or_else(|| task_error(format!("linked recoverable type row is absent for {ty:?}")))?;
    if matches.next().is_some() {
        return Err(task_error(format!(
            "linked recoverable type row is ambiguous for {ty:?}"
        )));
    }
    Ok(first)
}

fn runtime_type_plan_for_linked_type_index(
    image: &DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<RuntimeTypePlan, BytecodeTaskSubmitError> {
    let entry = checked_linked_type_entry(image, type_index)?;
    if let Some(carrier) = entry.representation_carrier() {
        let type_name = artifact_type_ref_label(entry.type_ref()).to_string();
        return Ok(RuntimeTypePlan::new(
            format!("linked representation {type_name}"),
            Some(type_name.clone()),
            RuntimeTypeNode::Representation {
                type_name,
                payload: Box::new(runtime_type_plan_for_linked_type_index(
                    image,
                    carrier.physical_carrier_type(),
                )?),
            },
        ));
    }
    runtime_type_plan_for_linked_type_ref(image, entry.type_ref())
}

fn runtime_type_plan_for_linked_type_ref(
    image: &DeploymentExecutionImage,
    ty: &TypeRefIr,
) -> Result<RuntimeTypePlan, BytecodeTaskSubmitError> {
    match ty {
        TypeRefIr::Builtin { name, args } => runtime_builtin_type_plan(image, name, args),
        TypeRefIr::Record { fields } => {
            let fields = fields
                .iter()
                .map(|(name, ty)| {
                    Ok(RuntimeRecordFieldPlan::new(
                        name.clone(),
                        runtime_type_plan_for_linked_type_ref(image, ty)?,
                        true,
                    ))
                })
                .collect::<Result<Vec<_>, BytecodeTaskSubmitError>>()?;
            Ok(RuntimeTypePlan::new(
                "linked inline record",
                None,
                RuntimeTypeNode::Record {
                    fields,
                    boundary_record_kind: None,
                },
            ))
        }
        TypeRefIr::Union { items } => Ok(RuntimeTypePlan::new(
            "linked union",
            None,
            RuntimeTypeNode::Union(
                items
                    .iter()
                    .map(|item| runtime_type_plan_for_linked_type_ref(image, item))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        )),
        TypeRefIr::Nullable { inner } => Ok(RuntimeTypePlan::synthetic_nullable(
            runtime_type_plan_for_linked_type_ref(image, inner)?,
        )),
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => Ok(RuntimeTypePlan::new(
            "linked string literal",
            None,
            RuntimeTypeNode::LiteralString(value.clone()),
        )),
        TypeRefIr::PackageSchema { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::AppliedNominal { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. } => {
            let type_index = unique_type_index_for_ref(image, ty)?;
            runtime_nominal_type_plan(image, type_index)
        }
        other => Err(task_error(format!(
            "linked recoverable task type is unsupported: {other:?}"
        ))),
    }
}

fn runtime_builtin_type_plan(
    image: &DeploymentExecutionImage,
    name: &str,
    args: &[TypeRefIr],
) -> Result<RuntimeTypePlan, BytecodeTaskSubmitError> {
    if name == "Array" && args.len() == 1 {
        return Ok(RuntimeTypePlan::synthetic_array(
            runtime_type_plan_for_linked_type_ref(image, &args[0])?,
        ));
    }
    if name == "Map" && args.len() == 2 {
        return Ok(RuntimeTypePlan::synthetic_map(
            runtime_type_plan_for_linked_type_ref(image, &args[0])?,
            runtime_type_plan_for_linked_type_ref(image, &args[1])?,
        ));
    }
    if name == "Stream" || !args.is_empty() {
        return Err(task_error(format!(
            "linked recoverable builtin {name} is unsupported"
        )));
    }
    Ok(
        skiff_runtime_boundary::db::runtime_type_plan_from_artifact_type_ref(&TypeRefIr::Builtin {
            name: name.to_string(),
            args: Vec::new(),
        }),
    )
}

fn runtime_nominal_type_plan(
    image: &DeploymentExecutionImage,
    type_index: TypeIndex,
) -> Result<RuntimeTypePlan, BytecodeTaskSubmitError> {
    let entry = checked_linked_type_entry(image, type_index)?;
    let shape = unique_shape_for_type_index(image.shapes(), type_index)?;
    let label = artifact_type_ref_label(entry.type_ref()).to_string();
    let fields = shape
        .fields()
        .iter()
        .map(|field| {
            Ok(RuntimeRecordFieldPlan::new(
                field.name().to_string(),
                runtime_type_plan_for_linked_type_index(image, field.ty())?,
                true,
            ))
        })
        .collect::<Result<Vec<_>, BytecodeTaskSubmitError>>()?;
    Ok(RuntimeTypePlan::new(
        label.clone(),
        Some(label.clone()),
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(label),
        },
    ))
}

fn unique_shape_for_type_index<'a>(
    shapes: &'a [LinkedShapeEntry],
    type_index: TypeIndex,
) -> Result<&'a LinkedShapeEntry, BytecodeTaskSubmitError> {
    let mut matches = shapes
        .iter()
        .filter(|shape| shape.nominal_type() == type_index);
    let first = matches.next().ok_or_else(|| {
        task_error(format!(
            "linked recoverable nominal type {} has no exact shape",
            type_index.get()
        ))
    })?;
    if matches.next().is_some() {
        return Err(task_error(format!(
            "linked recoverable nominal type {} matches more than one exact shape",
            type_index.get()
        )));
    }
    Ok(first)
}

pub(crate) fn task_payload_runtime_plan(
    image: &DeploymentExecutionImage,
    plan: &LinkedTaskPayloadPlan,
) -> Result<RuntimeTypePlan, BytecodeTaskSubmitError> {
    let fields = plan
        .parameters()
        .iter()
        .map(|parameter| {
            Ok(RuntimeRecordFieldPlan::new(
                parameter.name().to_string(),
                runtime_type_plan_for_linked_type_index(image, parameter.ty())?,
                true,
            ))
        })
        .collect::<Result<Vec<_>, BytecodeTaskSubmitError>>()?;
    Ok(RuntimeTypePlan::synthetic_request_record(fields))
}

fn task_payload_boundary(image: &DeploymentExecutionImage) -> PayloadBoundary {
    let deployment = image.owner().deployment();
    PayloadBoundary::owner_internal(PayloadBoundaryKind::TaskDispatchPayload).with_origin_service(
        PayloadServiceRef::new(deployment.service_id.clone())
            .with_version(deployment.contract_version.clone())
            .with_build_id(deployment.deployment_artifact_identity.to_string()),
    )
}

fn task_runtime_value_for_slot(
    heap: &RequestVmHeap,
    value: &ValueSlot,
) -> Result<RuntimeValue, BytecodeTaskSubmitError> {
    match value.kind() {
        Some(ValueKind::Null) => Ok(RuntimeValue::Null),
        Some(ValueKind::Bool) => {
            Ok(RuntimeValue::Bool(value.as_bool().ok_or_else(|| {
                task_error("task bool slot has no bool payload")
            })?))
        }
        Some(ValueKind::Number) => {
            Ok(RuntimeValue::Number(value.as_number().ok_or_else(
                || task_error("task number slot has no number payload"),
            )?))
        }
        Some(ValueKind::Integer) => Ok(RuntimeValue::Number(
            value
                .as_integer()
                .ok_or_else(|| task_error("task integer slot has no integer payload"))?
                as f64,
        )),
        Some(ValueKind::Date) => {
            Ok(RuntimeValue::Date(value.as_date().ok_or_else(|| {
                task_error("task date slot has no date payload")
            })?))
        }
        Some(ValueKind::RequestHeapRef) => heap
            .runtime_value_for_slot(value)
            .map_err(|error| task_error(format!("task heap value projection failed: {error}"))),
        other => Err(task_error(format!(
            "task slot kind {other:?} cannot enter a durable recoverable payload"
        ))),
    }
}

pub(crate) fn encode_durable_task_payload(
    image: &DeploymentExecutionImage,
    target: &LinkedTaskTarget,
    arguments: &[ValueSlot],
    heap: &mut dyn VmHeap,
) -> Result<Vec<u8>, BytecodeTaskSubmitError> {
    let plan = target
        .payload_plan()
        .map_err(|error| task_error(error.to_string()))?;
    let parameter_names = target
        .parameter_names()
        .map_err(|error| task_error(error.to_string()))?;
    if parameter_names.len() != arguments.len() || plan.parameter_count() != arguments.len() {
        return Err(task_error(format!(
            "task payload plan has {} parameter(s) but the dispatch supplied {} argument(s)",
            plan.parameter_count(),
            arguments.len()
        )));
    }
    for (name, parameter) in parameter_names.iter().zip(plan.parameters()) {
        if *name != parameter.name() {
            return Err(task_error(
                "linked task payload parameter names disagree with the exact plan",
            ));
        }
    }

    let request_heap = heap
        .as_any_mut()
        .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
        .ok_or_else(|| task_error("task child heap is not a request VM heap"))?;
    let runtime_plan = task_payload_runtime_plan(image, plan)?;
    let checkpoint = request_heap.request_heap().checkpoint();
    let encoded = (|| {
        let values = arguments
            .iter()
            .map(|value| task_runtime_value_for_slot(request_heap, value))
            .collect::<Result<Vec<_>, BytecodeTaskSubmitError>>()?;
        let mut object = RuntimeObjectFields::new();
        for (parameter, value) in plan.parameters().iter().zip(values) {
            object.insert(parameter.name().to_string(), value);
        }
        let handle = request_heap
            .request_heap_mut()
            .alloc_object(RuntimeObject::unshaped(object))
            .map_err(|error| task_error(format!("task record allocation failed: {error}")))?;
        let value = RuntimeValue::Heap(handle);
        let boundary = task_payload_boundary(image);
        encode_recoverable_payload_plan(
            &value,
            &runtime_plan,
            &boundary,
            request_heap.request_heap(),
        )
        .map_err(|error| task_error(format!("durable task payload encode failed: {error}")))
    })();
    request_heap
        .request_heap_mut()
        .rollback_to_checkpoint(checkpoint);
    encoded
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

#[cfg(test)]
pub(crate) fn task_child_failure_reason(
    image: &DeploymentExecutionImage,
    index: TaskDispatchIndex,
    composition: &BytecodeTaskChildComposition,
) -> String {
    match task_target_by_dispatch_index(image, index) {
        None => "task dispatch table row is absent".to_string(),
        Some(_target) if !composition.is_available() => {
            "task child requires exact host activation identity, caller request id and runtime id; \
             the current bytecode request has none (activation identity requires routing \
             assembly_identity + assembly_generation, deployment deployment_revision, and host \
             runtime_replica_id)"
                .to_string()
        }
        Some(target) => {
            if let Err(error) = target.payload_plan() {
                return error.to_string();
            }
            match task_timing_control(target) {
                Err(error) => error.to_string(),
                Ok(_) => "durable task child submitter is not installed".to_string(),
            }
        }
    }
}

pub(crate) fn task_submit_message_from_composition(
    deployment: &ServiceDeploymentRef,
    protocol_identity: &str,
    target: &LinkedTaskTarget,
    payload: &[u8],
    rpc_id: &str,
    composition: &BytecodeTaskChildComposition,
    timing: Option<TaskSubmitTimingControl>,
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
    let timing = timing.ok_or_else(|| {
        BytecodeTaskSubmitError::PortUnavailable(
            "task child requires a K6-resolved dispatch timing value".to_string(),
        )
    })?;
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
        CallableEffectSummary, DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId,
        ParamModeIr, ServiceDeploymentRef,
    };
    use skiff_compiler::{
        authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
        CompilerPlatformSources,
    };
    use skiff_runtime_boundary::binary::decode_recoverable_payload_plan;
    use skiff_runtime_linked_bytecode::{
        ArtifactShapeIndex, FunctionIndex, LinkedArtifactPoolOrigin, LinkedCallableSignature,
        LinkedShapeField, LinkedValueDropPlan, LinkedValueTransferPlan, ShapeIndex,
        TaskTargetIndex, TypeIndex,
    };
    use skiff_runtime_linker::{
        link_deployment_execution_image, DeploymentExecutionImage, LinkLimits,
    };
    use skiff_runtime_loader::{
        DeploymentBytecodeLoader, FilesystemDeploymentBytecodeContentResolver,
    };
    use skiff_runtime_model::{
        request_heap::{RequestHeap, RequestHeapLimits},
        runtime_value::HeapNode,
    };

    use crate::vm_heap::RequestVmHeap;

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
    fn duplicate_nominal_shape_fails_closed_for_task_payload_planning() {
        let origin = |index| {
            LinkedArtifactPoolOrigin::new(
                PackageBuildId::new("build:task"),
                ArtifactShapeIndex::new(index),
                None,
            )
            .expect("fixture shape origin is canonical")
        };
        let shape = |index: u32, plan: LinkedValueTransferPlan| {
            LinkedShapeEntry::new(
                ShapeIndex::new(index),
                origin(index),
                TypeIndex::new(0),
                plan,
                None,
                Box::new([
                    LinkedShapeField::new("value", TypeIndex::new(1), snapshot_plan())
                        .expect("fixture shape field is canonical"),
                ]),
            )
            .expect("fixture shape is canonical")
        };
        let shapes = vec![
            shape(
                0,
                LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::Trivial,
                },
            ),
            shape(
                1,
                LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                },
            ),
        ];

        let error = unique_shape_for_type_index(&shapes, TypeIndex::new(0))
            .expect_err("duplicate task payload shapes must fail closed");
        assert!(
            error
                .to_string()
                .contains("matches more than one exact shape"),
            "unexpected task shape ambiguity error: {error}"
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
            Some(TaskSubmitTimingControl::Immediate),
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
        assert_eq!(message.request.activation_identity, activation());
        assert!(message.request.actor_method.is_none());
    }

    #[test]
    fn task_child_failure_reason_reports_exact_activation_identity_seam() {
        let image = task_image();
        let target = image
            .task_targets()
            .next()
            .expect("task fixture links a task dispatch row")
            .clone();
        let reason = task_child_failure_reason(
            &image,
            TaskDispatchIndex::from_task_target_index(target.index())
                .expect("linked task target maps to a dispatch index"),
            &BytecodeTaskChildComposition::default(),
        );
        assert!(reason.contains("assembly_identity"));
        assert!(reason.contains("assembly_generation"));
        assert!(reason.contains("deployment_revision"));
        assert!(reason.contains("runtime_replica_id"));
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
            Some(TaskSubmitTimingControl::Immediate),
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
    fn task_submit_message_rejects_missing_timing() {
        let error = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::Immediate),
            b"payload",
            "rpc:task-child",
            &composition(),
            None,
        )
        .expect_err("missing timing must fail before submission");
        assert!(error.to_string().contains("timing"));
    }

    #[test]
    fn task_timing_control_rejects_unresolved_after_timing() {
        let error = task_timing_control(&target(LinkedTaskTiming::After { expression: 4 }))
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
            Some(TaskSubmitTimingControl::Immediate),
        )
        .expect_err("missing activation identity must fail closed");
        assert!(error.to_string().contains("activation identity"));
    }

    #[test]
    fn task_submit_message_fails_closed_without_caller_request_id() {
        let mut composition = composition();
        composition.caller_request_id.clear();
        let error = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::Immediate),
            b"payload",
            "rpc:task-child",
            &composition,
            Some(TaskSubmitTimingControl::Immediate),
        )
        .expect_err("missing caller request id must fail before submission");
        assert!(error.to_string().contains("caller request id"));
    }

    #[test]
    fn task_submit_message_fails_closed_without_runtime_id() {
        let mut composition = composition();
        composition.runtime_id.clear();
        let error = task_submit_message_from_composition(
            &deployment(),
            "protocol:task",
            &target(LinkedTaskTiming::Immediate),
            b"payload",
            "rpc:task-child",
            &composition,
            Some(TaskSubmitTimingControl::Immediate),
        )
        .expect_err("missing runtime id must fail before submission");
        assert!(error.to_string().contains("runtime id"));
    }

    #[test]
    fn task_payload_uses_exact_linked_record_parameter_names() {
        let image = task_image();
        let target = image
            .task_targets()
            .next()
            .expect("task fixture links a task dispatch row")
            .clone();
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());

        let bytes =
            encode_durable_task_payload(&image, &target, &[ValueSlot::number(7.0)], &mut heap)
                .expect("exact linked task payload must encode");
        let plan = task_payload_runtime_plan(&image, target.payload_plan().unwrap()).unwrap();
        let boundary = task_payload_boundary(&image);
        let mut decode_heap = RequestHeap::default();
        let decoded = decode_recoverable_payload_plan(&bytes, &plan, &boundary, &mut decode_heap)
            .expect("linked task payload must decode against the same exact plan");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("task payload must decode to a heap object");
        };
        let HeapNode::Object(object) = decode_heap.get(handle).expect("decoded object resolves")
        else {
            panic!("task payload must decode to an object");
        };
        assert_eq!(
            object.fields().get("value"),
            Some(&RuntimeValue::Number(7.0))
        );
    }

    #[test]
    fn task_payload_missing_plan_fails_closed() {
        let image = task_image();
        let target = target(LinkedTaskTiming::Immediate);
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());

        let error =
            encode_durable_task_payload(&image, &target, &[ValueSlot::number(7.0)], &mut heap)
                .expect_err("missing F6 payload plan must fail before submission");
        assert!(error.to_string().contains("payload plan"));
    }

    #[test]
    fn task_payload_uses_linked_tuple_parameter_names() {
        let image = task_image();
        let linked = image
            .task_targets()
            .next()
            .expect("task fixture links a task dispatch row")
            .clone();
        let parameter = skiff_runtime_linked_bytecode::LinkedTaskPayloadParameter::new(
            "value",
            linked.signature().parameter_types()[0],
            linked.signature().parameter_plans()[0].clone(),
        )
        .expect("linked tuple parameter name is canonical");
        let tuple_plan = LinkedTaskPayloadPlan::try_tuple(vec![parameter])
            .expect("linked tuple plan is canonical");
        let target = LinkedTaskTarget::new(
            linked.index(),
            linked.target_identity(),
            linked.function(),
            linked.signature().clone(),
            LinkedTaskTiming::Immediate,
        )
        .expect("linked tuple target is canonical")
        .with_payload_plan(tuple_plan)
        .expect("linked tuple target retains its payload plan");
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());

        let bytes =
            encode_durable_task_payload(&image, &target, &[ValueSlot::number(9.0)], &mut heap)
                .expect("exact linked tuple payload must encode with retained names");
        let plan = task_payload_runtime_plan(&image, target.payload_plan().unwrap()).unwrap();
        let boundary = task_payload_boundary(&image);
        let mut decode_heap = RequestHeap::default();
        let decoded = decode_recoverable_payload_plan(&bytes, &plan, &boundary, &mut decode_heap)
            .expect("linked tuple payload must decode against the same exact plan");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("tuple payload must decode to a heap object");
        };
        let HeapNode::Object(object) = decode_heap.get(handle).expect("decoded object resolves")
        else {
            panic!("tuple payload must decode to an object");
        };
        assert_eq!(
            object.fields().get("value"),
            Some(&RuntimeValue::Number(9.0))
        );
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

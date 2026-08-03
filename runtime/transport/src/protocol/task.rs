use serde::{Deserialize, Serialize};

use crate::{
    actor_method::{
        validate_actor_ref, validate_identity, validate_owner, validate_token,
        ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
    },
    protocol::{
        actor::ActivationIdentityFrameMetadata, decode_binary_frame, encode_binary_frame,
        request::RuntimeErrorFramePayload, FrameDirection, RUNTIME_FRAME_SCHEMA_VERSION,
    },
    BinaryFrameError, TransportError,
};

pub const TASK_SUBMIT_REQUEST_FRAME_TYPE: &str = "task.submit.request";
pub const TASK_SUBMIT_RESPONSE_FRAME_TYPE: &str = "task.submit.response";
pub const TASK_SUBMIT_ERROR_FRAME_TYPE: &str = "task.submit.error";
pub const TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED: &str = "submitted";
pub const TASK_CALLER_KIND_REQUEST: &str = "request";
pub const TASK_CALLER_KIND_ACTOR_INVOCATION: &str = "actorInvocation";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSubmitRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub target_kind: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub activation_identity: ActivationIdentityFrameMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queue_wait_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_method: Option<TaskActorMethodTargetFrameMetadata>,
}

/// Closed parent-kind namespace for the canonical task wire generation
/// (C-model-task §2). `callerKind` selects the unique parent resolver:
/// `request` -> FunctionTaskParentResolver, `actorInvocation` ->
/// ActorTaskParentResolver. The old shape (missing `callerKind`) is rejected
/// by the canonical codec with no compatible reader; the production consumer
/// hard cut is H-task-parent-cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskCallerKind {
    #[serde(rename = "request")]
    Request,
    #[serde(rename = "actorInvocation")]
    ActorInvocation,
}

/// Closed dispatch target classification (C-model-task §2): dispatch target kind is
/// orthogonal to `callerKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskTargetKind {
    #[serde(rename = "function")]
    Function,
    #[serde(rename = "actorMethod")]
    ActorMethod,
}

/// Canonical `task.submit.request` header (C-model-task §3.1).
///
/// Field order is part of the wire generation golden: it mirrors the frozen
/// corpus mirror so `encode_binary_frame` output is byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSubmitRequestFrameHeaderV2 {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub caller_kind: TaskCallerKind,
    pub caller_request_id: String,
    pub target_kind: TaskTargetKind,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub activation_identity: ActivationIdentityFrameMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queue_wait_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_method: Option<TaskActorMethodTargetFrameMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTaskSubmitRequestFrameHeaderV2 {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    rpc_id: String,
    runtime_id: String,
    caller_kind: TaskCallerKind,
    caller_request_id: String,
    target_kind: TaskTargetKind,
    service_id: String,
    service_version: String,
    service_protocol_identity: String,
    target: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    build_id: Option<String>,
    activation_identity: ActivationIdentityFrameMetadata,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    caller_target: Option<String>,
    #[serde(default)]
    max_queue_wait_ms: Option<f64>,
    #[serde(default)]
    actor_method: Option<TaskActorMethodTargetFrameMetadata>,
}

impl<'de> Deserialize<'de> for TaskSubmitRequestFrameHeaderV2 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawTaskSubmitRequestFrameHeaderV2::deserialize(deserializer)?;
        let header = Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            rpc_id: raw.rpc_id,
            runtime_id: raw.runtime_id,
            caller_kind: raw.caller_kind,
            caller_request_id: raw.caller_request_id,
            target_kind: raw.target_kind,
            service_id: raw.service_id,
            service_version: raw.service_version,
            service_protocol_identity: raw.service_protocol_identity,
            target: raw.target,
            task_id: raw.task_id,
            build_id: raw.build_id,
            activation_identity: raw.activation_identity,
            trace_id: raw.trace_id,
            caller_target: raw.caller_target,
            max_queue_wait_ms: raw.max_queue_wait_ms,
            actor_method: raw.actor_method,
        };
        validate_task_submit_request(&header).map_err(serde::de::Error::custom)?;
        Ok(header)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskActorMethodTargetFrameMetadata {
    pub actor_ref: ActorLogicalRefFrameHeader,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub actor_abi_identity: skiff_artifact_model::ActorAbiIdentity,
    pub actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity,
    pub method_identity: skiff_artifact_model::ActorMethodIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSubmitResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub task_id: String,
    pub request_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorTaskRuntimeErrorFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub error: RuntimeErrorFramePayload,
}

pub fn encode_task_submit_request_frame(
    header: &TaskSubmitRequestFrameHeaderV2,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_task_submit_request(header).map_err(TransportError::decode)?;
    encode_binary_frame(header, payload)
}

pub fn decode_task_submit_request_frame(
    bytes: &[u8],
) -> Result<(TaskSubmitRequestFrameHeaderV2, Vec<u8>), BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    let header: TaskSubmitRequestFrameHeaderV2 =
        serde_json::from_value(frame.header).map_err(|error| {
            TransportError::decode(format!("invalid task.submit.request: {error}"))
        })?;
    validate_task_submit_request(&header).map_err(TransportError::decode)?;
    Ok((header, frame.payload_bytes))
}

pub fn encode_task_submit_response_frame(
    header: &TaskSubmitResponseFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_response(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_submit_response_frame(
    bytes: &[u8],
) -> Result<TaskSubmitResponseFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_SUBMIT_RESPONSE_FRAME_TYPE)?;
    let header: TaskSubmitResponseFrameHeader =
        serde_json::from_value(frame.header).map_err(|error| {
            TransportError::decode(format!("invalid task.submit.response: {error}"))
        })?;
    validate_response(&header)?;
    Ok(header)
}

pub fn encode_task_submit_error_frame(
    header: &ActorTaskRuntimeErrorFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_error(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_submit_error_frame(
    bytes: &[u8],
) -> Result<ActorTaskRuntimeErrorFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_SUBMIT_ERROR_FRAME_TYPE)?;
    let header: ActorTaskRuntimeErrorFrameHeader = serde_json::from_value(frame.header)
        .map_err(|error| TransportError::decode(format!("invalid task.submit.error: {error}")))?;
    validate_error(&header)?;
    Ok(header)
}

/// Frame-level direction table for the task family (C-model-task §3.0).
///
/// The family is mixed-direction: the family-level registry marks `Either`,
/// but each task frame type has exactly one legal wire direction. Consumers
/// (demux, Runtime inbound handler) must narrow per frame; any other
/// direction is a protocol violation with no compatible reader.
pub fn task_submit_frame_direction(frame_type: &str) -> Option<FrameDirection> {
    match frame_type {
        TASK_SUBMIT_REQUEST_FRAME_TYPE => Some(FrameDirection::RuntimeToRouter),
        TASK_SUBMIT_RESPONSE_FRAME_TYPE | TASK_SUBMIT_ERROR_FRAME_TYPE => {
            Some(FrameDirection::RouterToRuntime)
        }
        _ => None,
    }
}

/// Canonical decoded `task.submit.request` (corpus `decodeAs:
/// "TaskSubmitRequest"`): the raw wire header plus the immutable opaque args
/// payload. This is the demux -> `TaskSubmitRouter` dispatch boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSubmitRequestFrame {
    pub header: TaskSubmitRequestFrameHeaderV2,
    pub payload: Vec<u8>,
}

/// Acceptance boundary for the stateless `TaskSubmitRouter`
/// (C-model-task §7.2 / C-task §3.3).
///
/// The acceptance carries the full decoded request (raw wire header + args
/// bytes) so the real execution sink can reconstruct the outbound
/// `task.submit.request` wire without re-parsing: service/activation
/// identity, `actorMethod` metadata and opaque args are all preserved
/// (E-actor-rust prerequisite). `request_id` is the Router-generated
/// correlation key for the accepted task's execution.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSubmitAcceptance {
    pub request: TaskSubmitRequestFrame,
    pub task_id: String,
    pub request_id: String,
}

impl TaskSubmitAcceptance {
    /// Typed projection of the Router->Runtime accept frame
    /// (`task.submit.response`, status `submitted`), echoing the request
    /// `rpcId` and carrying the Router-generated `taskId`/`requestId`.
    pub fn response_header(&self) -> TaskSubmitResponseFrameHeader {
        TaskSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: TASK_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
            rpc_id: self.request.header.rpc_id.clone(),
            task_id: self.task_id.clone(),
            request_id: self.request_id.clone(),
            status: TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
        }
    }
}

fn validate_task_submit_request(header: &TaskSubmitRequestFrameHeaderV2) -> Result<(), String> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_SUBMIT_REQUEST_FRAME_TYPE,
    )?;
    validate_token(&header.rpc_id, "rpcId")?;
    validate_token(&header.runtime_id, "runtimeId")?;
    validate_token(&header.caller_request_id, "callerRequestId")?;
    for (value, label) in [
        (&header.service_id, "serviceId"),
        (&header.service_version, "serviceVersion"),
        (&header.service_protocol_identity, "serviceProtocolIdentity"),
        (&header.target, "target"),
    ] {
        if value.trim().is_empty() {
            return Err(format!("{label} must be non-empty"));
        }
    }
    if let Some(task_id) = &header.task_id {
        validate_token(task_id, "taskId")?;
    }
    if let Some(build_id) = &header.build_id {
        validate_token(build_id, "buildId")?;
    }
    if let Some(caller_target) = &header.caller_target {
        validate_token(caller_target, "callerTarget")?;
    }
    if header
        .trace_id
        .as_deref()
        .is_some_and(|trace_id| trace_id.trim().is_empty())
    {
        return Err("traceId must be non-empty when present".into());
    }
    if let Some(max_queue_wait_ms) = header.max_queue_wait_ms {
        if !max_queue_wait_ms.is_finite()
            || max_queue_wait_ms <= 0.0
            || max_queue_wait_ms > 9_007_199_254_740_991.0
        {
            return Err("maxQueueWaitMs must be a positive JavaScript safe integer".into());
        }
    }
    match header.target_kind {
        TaskTargetKind::Function => {
            if header.actor_method.is_some() {
                return Err("targetKind function must not carry actorMethod".into());
            }
        }
        TaskTargetKind::ActorMethod => {
            let actor_method = header.actor_method.as_ref().ok_or_else(|| {
                "targetKind actorMethod requires actorMethod metadata".to_string()
            })?;
            validate_actor_ref(&actor_method.actor_ref)?;
            validate_owner(&actor_method.declaration_owner)?;
            validate_identity(
                actor_method.actor_abi_identity.as_str(),
                "skiff-actor-abi-v1:sha256",
            )?;
            validate_identity(
                actor_method.actor_implementation_identity.as_str(),
                "skiff-actor-implementation-v1:sha256",
            )?;
            validate_identity(
                actor_method.method_identity.as_str(),
                "skiff-actor-method-v1:sha256",
            )?;
        }
    }
    Ok(())
}

fn validate_response(header: &TaskSubmitResponseFrameHeader) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_SUBMIT_RESPONSE_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    validate_token(&header.task_id, "taskId").map_err(TransportError::decode)?;
    validate_token(&header.request_id, "requestId").map_err(TransportError::decode)?;
    if header.status != TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED {
        return Err(TransportError::decode(
            "task.submit.response status must be submitted",
        ));
    }
    Ok(())
}

fn validate_error(header: &ActorTaskRuntimeErrorFrameHeader) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_SUBMIT_ERROR_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    if header.error.code.trim().is_empty() {
        return Err(TransportError::decode(
            "task.submit.error code must be non-empty",
        ));
    }
    if header.error.message.trim().is_empty() || header.error.message.len() > 4096 {
        return Err(TransportError::decode(
            "task.submit.error message must contain 1..4096 bytes",
        ));
    }
    Ok(())
}

fn validate_common(schema: &str, actual: &str, expected: &str) -> Result<(), String> {
    if schema != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err("unsupported task frame schemaVersion".into());
    }
    if actual != expected {
        return Err(format!("task frame type must be {expected}"));
    }
    Ok(())
}

fn reject_payload(payload: &[u8], kind: &str) -> Result<(), BinaryFrameError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(TransportError::decode(format!(
            "{kind} payload must be empty"
        )))
    }
}

#[cfg(test)]
mod tests;

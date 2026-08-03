use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
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
pub const TASK_STATUS_REQUEST_FRAME_TYPE: &str = "task.status.request";
pub const TASK_STATUS_RESPONSE_FRAME_TYPE: &str = "task.status.response";
pub const TASK_CANCEL_REQUEST_FRAME_TYPE: &str = "task.cancel.request";
pub const TASK_CANCEL_RESPONSE_FRAME_TYPE: &str = "task.cancel.response";
pub const TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED: &str = "submitted";
pub const TASK_CALLER_KIND_REQUEST: &str = "request";
pub const TASK_CALLER_KIND_ACTOR_INVOCATION: &str = "actorInvocation";

/// Submission timing carried by `task.submit.request` (D1 wire contract).
///
/// `immediate` is the default/legacy kind and is omitted by the canonical
/// encoder (missing `timing` decodes as `Immediate`, keeping old corpora
/// byte-exact). `after` / `at` values are carried verbatim; negative or
/// overflow rejection semantics belong to the compiler/runtime, not the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TaskSubmitTiming {
    Immediate,
    After { duration_ms: u64 },
    At { utc_millis: i64 },
}

/// Canonical opaque task reference (D1 wire contract): encodes the `TaskId`
/// plus the owner scope so status / cancel / settlement can be recovered
/// across requests. Format:
/// `skiff-task-v1:<base64url-nopad(owner)>.<base64url-nopad(taskId)>`.
/// An undecodable reference is a wire error.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskRef {
    raw: String,
    owner: String,
    task_id: String,
}

impl TaskRef {
    pub const PREFIX: &'static str = "skiff-task-v1";

    pub fn new(task_id: impl Into<String>, owner: impl Into<String>) -> Result<Self, String> {
        let task_id = task_id.into();
        let owner = owner.into();
        if task_id.trim().is_empty() {
            return Err("taskRef taskId must be non-empty".to_string());
        }
        if owner.trim().is_empty() {
            return Err("taskRef owner scope must be non-empty".to_string());
        }
        Ok(Self::from_parts(owner, task_id))
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let rest = raw
            .strip_prefix(&format!("{}:", Self::PREFIX))
            .ok_or_else(|| format!("taskRef must start with {}:", Self::PREFIX))?;
        let (owner_encoded, task_encoded) = rest
            .split_once('.')
            .ok_or_else(|| "taskRef must be <owner>.<taskId> after the scheme".to_string())?;
        if owner_encoded.is_empty() || task_encoded.is_empty() {
            return Err("taskRef owner and taskId segments must be non-empty".to_string());
        }
        let owner = decode_task_ref_segment(owner_encoded, "owner")?;
        let task_id = decode_task_ref_segment(task_encoded, "taskId")?;
        if owner.trim().is_empty() {
            return Err("taskRef owner scope must be non-empty".to_string());
        }
        if task_id.trim().is_empty() {
            return Err("taskRef taskId must be non-empty".to_string());
        }
        Ok(Self::from_parts(owner, task_id))
    }

    fn from_parts(owner: String, task_id: String) -> Self {
        Self {
            raw: encode_task_ref(&owner, &task_id),
            owner,
            task_id,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn into_string(self) -> String {
        self.raw
    }
}

impl std::fmt::Display for TaskRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.raw)
    }
}

impl Serialize for TaskRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for TaskRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

fn encode_task_ref(owner: &str, task_id: &str) -> String {
    format!(
        "{}:{}.{}",
        TaskRef::PREFIX,
        URL_SAFE_NO_PAD.encode(owner),
        URL_SAFE_NO_PAD.encode(task_id)
    )
}

fn decode_task_ref_segment(encoded: &str, label: &str) -> Result<String, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| format!("taskRef {label} is not canonical base64url: {error}"))?;
    String::from_utf8(bytes)
        .map_err(|error| format!("taskRef {label} is not valid UTF-8: {error}"))
}

/// Wire projection of `std.task.status` kinds (`doc/reference/dispatch.md`
/// §3); strings match the canonical reference spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatusKindWire {
    Scheduled,
    Ready,
    Running,
    Succeeded,
    Failed,
    PlatformFailed,
    Canceled,
    Expired,
}

impl TaskStatusKindWire {
    pub const ALL: [Self; 8] = [
        Self::Scheduled,
        Self::Ready,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::PlatformFailed,
        Self::Canceled,
        Self::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::PlatformFailed => "platformFailed",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
        }
    }
}

/// Wire projection of `std.task.status` result (D1 wire contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskStatusWire {
    pub kind: TaskStatusKindWire,
}

/// Wire projection of `std.task.cancel` result kinds (`doc/reference/dispatch.md`
/// §3); strings match the canonical reference spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskCancelResultKindWire {
    Canceled,
    AlreadyStarted,
    AlreadyTerminal,
    Expired,
}

impl TaskCancelResultKindWire {
    pub const ALL: [Self; 4] = [
        Self::Canceled,
        Self::AlreadyStarted,
        Self::AlreadyTerminal,
        Self::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Canceled => "canceled",
            Self::AlreadyStarted => "alreadyStarted",
            Self::AlreadyTerminal => "alreadyTerminal",
            Self::Expired => "expired",
        }
    }
}

/// Wire projection of `std.task.cancel` result (D1 wire contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskCancelResultWire {
    pub kind: TaskCancelResultKindWire,
}

/// Canonical `task.submit.error` rejection-code vocabulary (D1 wire contract).
///
/// `storeUnavailable` is a transient failure (no task was created; a later
/// submission may succeed). The other codes are definite rejections: a
/// successful `task.submit.response` guarantees a durable task, while these
/// guarantee no task was created by that submission. Existing router parent /
/// authority error strings remain accepted by the wire so current runtime
/// behavior is unchanged; the control plane (D2) emits this vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskSubmitRejectionCode {
    InvalidTiming,
    PayloadInvalid,
    QuotaExceeded,
    StoreUnavailable,
    Rejected,
}

impl TaskSubmitRejectionCode {
    pub const ALL: [Self; 5] = [
        Self::InvalidTiming,
        Self::PayloadInvalid,
        Self::QuotaExceeded,
        Self::StoreUnavailable,
        Self::Rejected,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidTiming => "invalidTiming",
            Self::PayloadInvalid => "payloadInvalid",
            Self::QuotaExceeded => "quotaExceeded",
            Self::StoreUnavailable => "storeUnavailable",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|candidate| candidate.as_str() == code)
    }

    /// `storeUnavailable` is the transient failure; every other code is a
    /// definite rejection with no task created.
    pub fn is_transient(self) -> bool {
        matches!(self, Self::StoreUnavailable)
    }

    pub fn is_definite(self) -> bool {
        !self.is_transient()
    }
}

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
    pub timing: Option<TaskSubmitTiming>,
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
    pub timing: Option<TaskSubmitTiming>,
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
    timing: Option<TaskSubmitTiming>,
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
            timing: raw.timing,
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
    pub task_ref: TaskRef,
    pub task_id: String,
    pub request_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskStatusRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub task_ref: TaskRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskStatusResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub task_ref: TaskRef,
    pub status: TaskStatusWire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskCancelRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub task_ref: TaskRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskCancelResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub task_ref: TaskRef,
    pub result: TaskCancelResultWire,
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

pub fn encode_task_status_request_frame(
    header: &TaskStatusRequestFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_status_request(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_status_request_frame(
    bytes: &[u8],
) -> Result<TaskStatusRequestFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_STATUS_REQUEST_FRAME_TYPE)?;
    let header: TaskStatusRequestFrameHeader = serde_json::from_value(frame.header).map_err(
        |error| TransportError::decode(format!("invalid task.status.request: {error}")),
    )?;
    validate_status_request(&header)?;
    Ok(header)
}

pub fn encode_task_status_response_frame(
    header: &TaskStatusResponseFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_status_response(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_status_response_frame(
    bytes: &[u8],
) -> Result<TaskStatusResponseFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_STATUS_RESPONSE_FRAME_TYPE)?;
    let header: TaskStatusResponseFrameHeader = serde_json::from_value(frame.header).map_err(
        |error| TransportError::decode(format!("invalid task.status.response: {error}")),
    )?;
    validate_status_response(&header)?;
    Ok(header)
}

pub fn encode_task_cancel_request_frame(
    header: &TaskCancelRequestFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_cancel_request(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_cancel_request_frame(
    bytes: &[u8],
) -> Result<TaskCancelRequestFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_CANCEL_REQUEST_FRAME_TYPE)?;
    let header: TaskCancelRequestFrameHeader = serde_json::from_value(frame.header).map_err(
        |error| TransportError::decode(format!("invalid task.cancel.request: {error}")),
    )?;
    validate_cancel_request(&header)?;
    Ok(header)
}

pub fn encode_task_cancel_response_frame(
    header: &TaskCancelResponseFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_cancel_response(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_task_cancel_response_frame(
    bytes: &[u8],
) -> Result<TaskCancelResponseFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    reject_payload(&frame.payload_bytes, TASK_CANCEL_RESPONSE_FRAME_TYPE)?;
    let header: TaskCancelResponseFrameHeader = serde_json::from_value(frame.header).map_err(
        |error| TransportError::decode(format!("invalid task.cancel.response: {error}")),
    )?;
    validate_cancel_response(&header)?;
    Ok(header)
}

/// Frame-level direction table for the task family (C-model-task §3.0).
///
/// The family is mixed-direction: the family-level registry marks `Either`,
/// but each task frame type has exactly one legal wire direction. Since D1
/// the table covers the whole `task.*` family (submit + status/cancel).
/// Consumers (demux, Runtime inbound handler) must narrow per frame; any
/// other direction is a protocol violation with no compatible reader.
pub fn task_submit_frame_direction(frame_type: &str) -> Option<FrameDirection> {
    match frame_type {
        TASK_SUBMIT_REQUEST_FRAME_TYPE
        | TASK_STATUS_REQUEST_FRAME_TYPE
        | TASK_CANCEL_REQUEST_FRAME_TYPE => Some(FrameDirection::RuntimeToRouter),
        TASK_SUBMIT_RESPONSE_FRAME_TYPE
        | TASK_SUBMIT_ERROR_FRAME_TYPE
        | TASK_STATUS_RESPONSE_FRAME_TYPE
        | TASK_CANCEL_RESPONSE_FRAME_TYPE => Some(FrameDirection::RouterToRuntime),
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
    /// `rpcId` and carrying the Router-generated
    /// `taskRef`/`taskId`/`requestId`. The owner scope is the accepted
    /// request's authenticated service id (D1 wire contract).
    pub fn response_header(&self) -> TaskSubmitResponseFrameHeader {
        TaskSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: TASK_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
            rpc_id: self.request.header.rpc_id.clone(),
            task_ref: TaskRef::new(
                self.task_id.clone(),
                self.request.header.service_id.clone(),
            )
            .expect("accepted taskId and serviceId are non-empty"),
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

fn validate_status_request(
    header: &TaskStatusRequestFrameHeader,
) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_STATUS_REQUEST_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    TaskRef::parse(header.task_ref.as_str()).map_err(TransportError::decode)?;
    Ok(())
}

fn validate_status_response(
    header: &TaskStatusResponseFrameHeader,
) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_STATUS_RESPONSE_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    TaskRef::parse(header.task_ref.as_str()).map_err(TransportError::decode)?;
    Ok(())
}

fn validate_cancel_request(
    header: &TaskCancelRequestFrameHeader,
) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_CANCEL_REQUEST_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    TaskRef::parse(header.task_ref.as_str()).map_err(TransportError::decode)?;
    Ok(())
}

fn validate_cancel_response(
    header: &TaskCancelResponseFrameHeader,
) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        TASK_CANCEL_RESPONSE_FRAME_TYPE,
    )
    .map_err(TransportError::decode)?;
    validate_token(&header.rpc_id, "rpcId").map_err(TransportError::decode)?;
    TaskRef::parse(header.task_ref.as_str()).map_err(TransportError::decode)?;
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

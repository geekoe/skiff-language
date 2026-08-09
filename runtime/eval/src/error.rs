use std::{fmt, sync::Arc};

use serde::Serialize;
use serde_json::{json, Map, Value};
use skiff_runtime_model::request_heap::{
    deep_clone_runtime_value_carrier_between_heaps, RequestHeap,
};
use skiff_runtime_model::runtime_value::RuntimeValueCarrier;

pub use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{
        CatchIdentity, InstantiatedTypeArgumentIdentity, OpaqueServiceError,
        PlatformBuiltinErrorIdentity, RequestException, RequestExceptionCause,
    },
};

mod scope_terminal;
pub(crate) use scope_terminal::ScopeTerminalCarrier;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSource {
    pub assembly_id: Option<u32>,
    pub source_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetReason {
    Cancelled,
    DeadlineExceeded,
    InstructionLimitExceeded,
}

impl BudgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetReason::Cancelled => "cancelled",
            BudgetReason::DeadlineExceeded => "deadlineExceeded",
            BudgetReason::InstructionLimitExceeded => "instructionLimitExceeded",
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// Eval cancellation is an internal execution terminal, not a wire error.
///
/// ```compile_fail
/// use skiff_runtime_eval::error::{RuntimeError, WirePayload};
///
/// let _ = WirePayload::payload(&RuntimeError::Cancelled);
/// ```
pub enum RuntimeError {
    #[error(transparent)]
    ActorInstance(#[from] crate::actor_instance::ActorInstanceStoreError),
    #[error("{0}")]
    InvalidArtifact(String),
    #[error("{0}")]
    Decode(String),
    #[error("decode error for {target}: {message}")]
    DecodeTarget { target: String, message: String },
    #[error("bytes decode error for {target}: {message}")]
    BytesDecode { target: String, message: String },
    #[error("db decode error for {target}: {message}")]
    DbDecode { target: String, message: String },
    #[error("file error: {message}")]
    FileError { message: String },
    #[error("resource error for {path}: {message}")]
    ResourceError { path: String, message: String },
    #[error("http error: {message}")]
    HttpError {
        message: String,
        detail: Option<Value>,
    },
    #[error("{0}")]
    Unsupported(String),
    #[error(transparent)]
    Recoverable(#[from] skiff_runtime_boundary::error::RecoverableBoundaryError),
    #[error("db lease lost: {0}")]
    LeaseLost(String),
    #[error("request was cancelled")]
    Cancelled,
    #[error(transparent)]
    ScopeTerminal(ScopeTerminalCarrier),
    #[error("execution budget exceeded: {reason:?}")]
    ExecutionBudgetExceeded {
        reason: BudgetReason,
        instruction_count: u64,
        limit: Option<u64>,
        elapsed_ms: f64,
    },
    #[error("resource limit exceeded for {resource}: {reason}")]
    #[allow(dead_code)]
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
    #[error("unhandled user exception {0}")]
    UserException(UserException),
    /// A strict, already-fixed service failure. Its display text is deliberately
    /// constant so raw payload bytes never enter generic diagnostics.
    #[error("canonical service failure")]
    FixedServiceFailure(OpaqueServiceError),
    /// Runtime-owned failure to durably submit a `dispatch` task.
    ///
    /// This must never acquire an ordinary catch projection: task submission
    /// is part of the Runtime implementation of `dispatch`, not a business
    /// effect that Skiff code can recover or retry itself.
    #[error("task dispatch failed for {target}: {reason}")]
    TaskDispatchFailure { target: String, reason: String },
    #[error("provider unavailable for {target}: {reason}")]
    ProviderUnavailable { target: String, reason: String },
    #[error("protocol error for {target}: {message}")]
    #[allow(dead_code)]
    Protocol { target: String, message: String },
    #[error("{error}")]
    WithSource {
        source_id: u64,
        frame: Box<Value>,
        error: Box<RuntimeError>,
    },
    #[error("{error}")]
    WithDiagnosticFrame {
        frame: Box<Value>,
        error: Box<RuntimeError>,
    },
    #[error("{0}")]
    RootRuntimePayload(RuntimeErrorPayload),
    #[error("{0}")]
    Opaque(Box<dyn WirePayload>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl From<skiff_runtime_model::error::RuntimeModelError> for RuntimeError {
    fn from(error: skiff_runtime_model::error::RuntimeModelError) -> Self {
        match error {
            skiff_runtime_model::error::RuntimeModelError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            },
            skiff_runtime_model::error::RuntimeModelError::Json(error) => RuntimeError::Json(error),
        }
    }
}

impl From<skiff_runtime_boundary::error::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_boundary::error::RuntimeError) -> Self {
        match error {
            skiff_runtime_boundary::error::RuntimeError::InvalidArtifact(message) => {
                RuntimeError::InvalidArtifact(message)
            }
            skiff_runtime_boundary::error::RuntimeError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_boundary::error::RuntimeError::DecodeTarget { target, message } => {
                RuntimeError::DecodeTarget { target, message }
            }
            skiff_runtime_boundary::error::RuntimeError::BytesDecode { target, message } => {
                RuntimeError::BytesDecode { target, message }
            }
            skiff_runtime_boundary::error::RuntimeError::DbDecode { target, message } => {
                RuntimeError::DbDecode { target, message }
            }
            skiff_runtime_boundary::error::RuntimeError::FileError { message } => {
                RuntimeError::FileError { message }
            }
            skiff_runtime_boundary::error::RuntimeError::HttpError { message, detail } => {
                RuntimeError::HttpError { message, detail }
            }
            skiff_runtime_boundary::error::RuntimeError::Unsupported(message) => {
                RuntimeError::Unsupported(message)
            }
            skiff_runtime_boundary::error::RuntimeError::Recoverable(error) => {
                RuntimeError::Recoverable(error)
            }
            skiff_runtime_boundary::error::RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            },
            skiff_runtime_boundary::error::RuntimeError::Json(error) => RuntimeError::Json(error),
        }
    }
}

impl From<skiff_runtime_linked_type_plan::Error> for RuntimeError {
    fn from(error: skiff_runtime_linked_type_plan::Error) -> Self {
        match error {
            skiff_runtime_linked_type_plan::Error::InvalidArtifact(message) => {
                RuntimeError::InvalidArtifact(message)
            }
            skiff_runtime_linked_type_plan::Error::Protocol { target, message } => {
                RuntimeError::Protocol { target, message }
            }
            skiff_runtime_linked_type_plan::Error::Boundary(error) => RuntimeError::from(*error),
        }
    }
}

impl From<skiff_runtime_native::error::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_native::error::RuntimeError) -> Self {
        match error {
            skiff_runtime_native::error::RuntimeError::InvalidArtifact(message) => {
                RuntimeError::InvalidArtifact(message)
            }
            skiff_runtime_native::error::RuntimeError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_native::error::RuntimeError::DecodeTarget { target, message } => {
                RuntimeError::DecodeTarget { target, message }
            }
            skiff_runtime_native::error::RuntimeError::BytesDecode { target, message } => {
                RuntimeError::BytesDecode { target, message }
            }
            skiff_runtime_native::error::RuntimeError::DbDecode { target, message } => {
                RuntimeError::DbDecode { target, message }
            }
            skiff_runtime_native::error::RuntimeError::FileError { message } => {
                RuntimeError::FileError { message }
            }
            skiff_runtime_native::error::RuntimeError::ResourceError { path, message } => {
                RuntimeError::ResourceError { path, message }
            }
            skiff_runtime_native::error::RuntimeError::HttpError { message, detail } => {
                RuntimeError::HttpError { message, detail }
            }
            skiff_runtime_native::error::RuntimeError::Unsupported(message) => {
                RuntimeError::Unsupported(message)
            }
            skiff_runtime_native::error::RuntimeError::Recoverable(error) => {
                RuntimeError::Recoverable(error)
            }
            skiff_runtime_native::error::RuntimeError::Cancelled => RuntimeError::Cancelled,
            skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => RuntimeError::ExecutionBudgetExceeded {
                reason: runtime_budget_reason(reason),
                instruction_count,
                limit,
                elapsed_ms,
            },
            skiff_runtime_native::error::RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            },
            skiff_runtime_native::error::RuntimeError::Opaque(error) => {
                runtime_error_from_wire_payload(error)
            }
            skiff_runtime_native::error::RuntimeError::Json(error) => RuntimeError::Json(error),
        }
    }
}

fn runtime_budget_reason(reason: skiff_runtime_native::error::BudgetReason) -> BudgetReason {
    match reason {
        skiff_runtime_native::error::BudgetReason::Cancelled => BudgetReason::Cancelled,
        skiff_runtime_native::error::BudgetReason::DeadlineExceeded => {
            BudgetReason::DeadlineExceeded
        }
        skiff_runtime_native::error::BudgetReason::InstructionLimitExceeded => {
            BudgetReason::InstructionLimitExceeded
        }
    }
}

fn capability_budget_reason_to_eval(
    reason: skiff_runtime_capability_context::ExecutionBudgetReason,
) -> BudgetReason {
    match reason {
        skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled => {
            BudgetReason::Cancelled
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded => {
            BudgetReason::DeadlineExceeded
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded => {
            BudgetReason::InstructionLimitExceeded
        }
    }
}

/// Convert an opaque wire carrier arriving from the host root into an eval
/// error.
///
/// Carriers stay `RuntimeError::Opaque` by default. Only ordinary carriers
/// whose semantics eval consumes structurally are unpacked. Cancellation never
/// enters this API: mixed runtime errors must first pass an ordinary-only
/// wrapper, whose constructor rejects the internal terminal.
fn runtime_error_from_wire_payload(error: Box<dyn WirePayload>) -> RuntimeError {
    if let Some(error) = error.as_any().downcast_ref::<OrdinaryRuntimeError>() {
        return runtime_error_from_eval_ref(error.error());
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return runtime_error_from_model_ref(error);
    }
    if let Some(skiff_runtime_boundary::error::RuntimeError::Recoverable(recoverable)) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::error::RuntimeError>()
    {
        return RuntimeError::Recoverable(recoverable.clone());
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_native::error::OrdinaryRuntimeError>()
    {
        return runtime_error_from_native_ref(error.error());
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_linked_type_plan::Error>()
    {
        return runtime_error_from_linked_type_plan_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::CapabilityError>()
    {
        return runtime_error_from_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::DbCapabilityError>()
    {
        return runtime_error_from_db_capability_ref(error);
    }
    RuntimeError::Opaque(error)
}

fn runtime_error_from_model_ref(
    error: &skiff_runtime_model::error::RuntimeModelError,
) -> RuntimeError {
    match error {
        skiff_runtime_model::error::RuntimeModelError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => RuntimeError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        skiff_runtime_model::error::RuntimeModelError::Json(_) => {
            RuntimeError::Decode(error.to_string())
        }
    }
}

fn runtime_error_from_eval_ref(error: &RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::ActorInstance(error) => RuntimeError::InvalidArtifact(error.to_string()),
        RuntimeError::InvalidArtifact(message) => RuntimeError::InvalidArtifact(message.clone()),
        RuntimeError::Decode(message) => RuntimeError::Decode(message.clone()),
        RuntimeError::DecodeTarget { target, message } => RuntimeError::DecodeTarget {
            target: target.clone(),
            message: message.clone(),
        },
        RuntimeError::BytesDecode { target, message } => RuntimeError::BytesDecode {
            target: target.clone(),
            message: message.clone(),
        },
        RuntimeError::DbDecode { target, message } => RuntimeError::DbDecode {
            target: target.clone(),
            message: message.clone(),
        },
        RuntimeError::FileError { message } => RuntimeError::FileError {
            message: message.clone(),
        },
        RuntimeError::ResourceError { path, message } => RuntimeError::ResourceError {
            path: path.clone(),
            message: message.clone(),
        },
        RuntimeError::HttpError { message, detail } => RuntimeError::HttpError {
            message: message.clone(),
            detail: detail.clone(),
        },
        RuntimeError::Unsupported(message) => RuntimeError::Unsupported(message.clone()),
        RuntimeError::Recoverable(error) => RuntimeError::Recoverable(error.clone()),
        RuntimeError::LeaseLost(message) => RuntimeError::LeaseLost(message.clone()),
        RuntimeError::Cancelled => RuntimeError::Cancelled,
        RuntimeError::ScopeTerminal(terminal) => RuntimeError::ScopeTerminal(terminal.clone()),
        RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => RuntimeError::ExecutionBudgetExceeded {
            reason: *reason,
            instruction_count: *instruction_count,
            limit: *limit,
            elapsed_ms: *elapsed_ms,
        },
        RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => RuntimeError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        RuntimeError::UserException(error) => RuntimeError::UserException(error.clone()),
        RuntimeError::FixedServiceFailure(error) => {
            RuntimeError::FixedServiceFailure(error.clone())
        }
        RuntimeError::TaskDispatchFailure { target, reason } => RuntimeError::TaskDispatchFailure {
            target: target.clone(),
            reason: reason.clone(),
        },
        RuntimeError::ProviderUnavailable { target, reason } => RuntimeError::ProviderUnavailable {
            target: target.clone(),
            reason: reason.clone(),
        },
        RuntimeError::Protocol { target, message } => RuntimeError::Protocol {
            target: target.clone(),
            message: message.clone(),
        },
        RuntimeError::WithSource {
            source_id,
            frame,
            error,
        } => RuntimeError::WithSource {
            source_id: *source_id,
            frame: frame.clone(),
            error: Box::new(runtime_error_from_eval_ref(error)),
        },
        RuntimeError::WithDiagnosticFrame { frame, error } => RuntimeError::WithDiagnosticFrame {
            frame: frame.clone(),
            error: Box::new(runtime_error_from_eval_ref(error)),
        },
        RuntimeError::RootRuntimePayload(payload) => {
            RuntimeError::RootRuntimePayload(payload.clone())
        }
        RuntimeError::Opaque(error) => runtime_error_from_wire_payload_ref(error.as_ref()),
        RuntimeError::Json(_) => RuntimeError::Decode(error.to_string()),
    }
}

fn runtime_error_from_boundary_ref(
    error: &skiff_runtime_boundary::error::RuntimeError,
) -> RuntimeError {
    match error {
        skiff_runtime_boundary::error::RuntimeError::InvalidArtifact(message) => {
            RuntimeError::InvalidArtifact(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::DecodeTarget { target, message } => {
            RuntimeError::DecodeTarget {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::error::RuntimeError::BytesDecode { target, message } => {
            RuntimeError::BytesDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::error::RuntimeError::DbDecode { target, message } => {
            RuntimeError::DbDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::error::RuntimeError::FileError { message } => {
            RuntimeError::FileError {
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::error::RuntimeError::HttpError { message, detail } => {
            RuntimeError::HttpError {
                message: message.clone(),
                detail: detail.clone(),
            }
        }
        skiff_runtime_boundary::error::RuntimeError::Unsupported(message) => {
            RuntimeError::Unsupported(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::Recoverable(error) => {
            RuntimeError::Recoverable(error.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => RuntimeError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        skiff_runtime_boundary::error::RuntimeError::Json(_) => {
            RuntimeError::Decode(error.to_string())
        }
    }
}

fn runtime_error_from_native_ref(
    error: &skiff_runtime_native::error::RuntimeError,
) -> RuntimeError {
    match error {
        skiff_runtime_native::error::RuntimeError::InvalidArtifact(message) => {
            RuntimeError::InvalidArtifact(message.clone())
        }
        skiff_runtime_native::error::RuntimeError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_native::error::RuntimeError::DecodeTarget { target, message } => {
            RuntimeError::DecodeTarget {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::BytesDecode { target, message } => {
            RuntimeError::BytesDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::DbDecode { target, message } => {
            RuntimeError::DbDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::FileError { message } => {
            RuntimeError::FileError {
                message: message.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::ResourceError { path, message } => {
            RuntimeError::ResourceError {
                path: path.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::HttpError { message, detail } => {
            RuntimeError::HttpError {
                message: message.clone(),
                detail: detail.clone(),
            }
        }
        skiff_runtime_native::error::RuntimeError::Unsupported(message) => {
            RuntimeError::Unsupported(message.clone())
        }
        skiff_runtime_native::error::RuntimeError::Recoverable(error) => {
            RuntimeError::Recoverable(error.clone())
        }
        skiff_runtime_native::error::RuntimeError::Cancelled => RuntimeError::Cancelled,
        skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => RuntimeError::ExecutionBudgetExceeded {
            reason: runtime_budget_reason(*reason),
            instruction_count: *instruction_count,
            limit: *limit,
            elapsed_ms: *elapsed_ms,
        },
        skiff_runtime_native::error::RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => RuntimeError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        skiff_runtime_native::error::RuntimeError::Opaque(error) => {
            runtime_error_from_wire_payload_ref(error.as_ref())
        }
        skiff_runtime_native::error::RuntimeError::Json(_) => {
            RuntimeError::Decode(error.to_string())
        }
    }
}

fn runtime_error_from_wire_payload_ref(error: &dyn WirePayload) -> RuntimeError {
    if let Some(error) = error.as_any().downcast_ref::<OrdinaryRuntimeError>() {
        return runtime_error_from_eval_ref(error.error());
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return runtime_error_from_model_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::error::RuntimeError>()
    {
        return runtime_error_from_boundary_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_native::error::OrdinaryRuntimeError>()
    {
        return runtime_error_from_native_ref(error.error());
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_linked_type_plan::Error>()
    {
        return runtime_error_from_linked_type_plan_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::CapabilityError>()
    {
        return runtime_error_from_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::DbCapabilityError>()
    {
        return runtime_error_from_db_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::RequestPayloadContextError>()
    {
        return runtime_error_from_request_payload_ref(error);
    }
    if let Some(error) = error.as_any().downcast_ref::<RequestHeapOwnedStreamError>() {
        return RuntimeError::Opaque(Box::new(error.clone()));
    }
    RuntimeError::Decode(error.to_string())
}

fn runtime_error_from_linked_type_plan_ref(
    error: &skiff_runtime_linked_type_plan::Error,
) -> RuntimeError {
    match error {
        skiff_runtime_linked_type_plan::Error::InvalidArtifact(message) => {
            RuntimeError::InvalidArtifact(message.clone())
        }
        skiff_runtime_linked_type_plan::Error::Protocol { target, message } => {
            RuntimeError::Protocol {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_linked_type_plan::Error::Boundary(error) => {
            runtime_error_from_boundary_ref(error)
        }
    }
}

fn runtime_error_from_capability_ref(
    error: &skiff_runtime_capability_context::CapabilityError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::CapabilityError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_capability_context::CapabilityError::Unsupported(message) => {
            RuntimeError::Unsupported(message.clone())
        }
        skiff_runtime_capability_context::CapabilityError::ProviderUnavailable {
            target,
            reason,
        } => RuntimeError::ProviderUnavailable {
            target: target.clone(),
            reason: reason.clone(),
        },
        skiff_runtime_capability_context::CapabilityError::Protocol { target, message } => {
            RuntimeError::Protocol {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_capability_context::CapabilityError::TaskSubmitRejected { code, message } => {
            RuntimeError::ProviderUnavailable {
                target: "task.submit.request".to_string(),
                reason: format!("task.submit rejected ({code}): {message}"),
            }
        }
        skiff_runtime_capability_context::CapabilityError::TaskControlRejected {
            code,
            message,
        } => RuntimeError::ProviderUnavailable {
            target: "task.status.request".to_string(),
            reason: format!("task control rejected ({code}): {message}"),
        },
        skiff_runtime_capability_context::CapabilityError::Opaque(error) => {
            runtime_error_from_wire_payload_ref(error.as_ref())
        }
    }
}

fn runtime_error_from_db_capability_ref(
    error: &skiff_runtime_capability_context::DbCapabilityError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::DbCapabilityError::Decode(message) => {
            RuntimeError::DbDecode {
                target: "std.db".to_string(),
                message: message.clone(),
            }
        }
        skiff_runtime_capability_context::DbCapabilityError::ProviderUnavailable {
            target,
            reason,
        } => RuntimeError::ProviderUnavailable {
            target: target.clone(),
            reason: reason.clone(),
        },
        skiff_runtime_capability_context::DbCapabilityError::Opaque(error) => {
            runtime_error_from_wire_payload_ref(error.as_ref())
        }
    }
}

fn runtime_error_from_request_payload_ref(
    error: &skiff_runtime_capability_context::RequestPayloadContextError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
            target,
        } => RuntimeError::Protocol {
            target: target.clone(),
            message: error.to_string(),
        },
    }
}

pub fn eval_error_to_native(error: RuntimeError) -> skiff_runtime_native::error::RuntimeError {
    match error {
        RuntimeError::InvalidArtifact(message) => {
            skiff_runtime_native::error::RuntimeError::InvalidArtifact(message)
        }
        RuntimeError::Decode(message) => skiff_runtime_native::error::RuntimeError::Decode(message),
        RuntimeError::DecodeTarget { target, message } => {
            skiff_runtime_native::error::RuntimeError::DecodeTarget { target, message }
        }
        RuntimeError::BytesDecode { target, message } => {
            skiff_runtime_native::error::RuntimeError::BytesDecode { target, message }
        }
        RuntimeError::DbDecode { target, message } => {
            skiff_runtime_native::error::RuntimeError::DbDecode { target, message }
        }
        RuntimeError::FileError { message } => {
            skiff_runtime_native::error::RuntimeError::FileError { message }
        }
        RuntimeError::ResourceError { path, message } => {
            skiff_runtime_native::error::RuntimeError::ResourceError { path, message }
        }
        RuntimeError::HttpError { message, detail } => {
            skiff_runtime_native::error::RuntimeError::HttpError { message, detail }
        }
        RuntimeError::Unsupported(message) => {
            skiff_runtime_native::error::RuntimeError::Unsupported(message)
        }
        RuntimeError::Recoverable(error) => {
            skiff_runtime_native::error::RuntimeError::Recoverable(error)
        }
        RuntimeError::Cancelled => skiff_runtime_native::error::RuntimeError::Cancelled,
        RuntimeError::ScopeTerminal(_) => skiff_runtime_native::error::RuntimeError::Cancelled,
        RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
            reason: native_budget_reason(reason),
            instruction_count,
            limit,
            elapsed_ms,
        },
        RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => skiff_runtime_native::error::RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        },
        RuntimeError::Json(error) => skiff_runtime_native::error::RuntimeError::Json(error),
        RuntimeError::Opaque(error) => skiff_runtime_native::error::RuntimeError::Opaque(error),
        error => skiff_runtime_native::error::RuntimeError::Opaque(Box::new(
            OrdinaryRuntimeError::try_new(error)
                .expect("cancellation is converted before ordinary trait erasure"),
        )),
    }
}

fn native_budget_reason(reason: BudgetReason) -> skiff_runtime_native::error::BudgetReason {
    match reason {
        BudgetReason::Cancelled => skiff_runtime_native::error::BudgetReason::Cancelled,
        BudgetReason::DeadlineExceeded => {
            skiff_runtime_native::error::BudgetReason::DeadlineExceeded
        }
        BudgetReason::InstructionLimitExceeded => {
            skiff_runtime_native::error::BudgetReason::InstructionLimitExceeded
        }
    }
}

impl From<skiff_runtime_capability_context::ExecutionControlError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::ExecutionControlError) -> Self {
        match error {
            skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
                RuntimeError::Cancelled
            }
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
                if failure.reason
                    == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
                {
                    RuntimeError::Cancelled
                } else {
                    RuntimeError::ExecutionBudgetExceeded {
                        reason: capability_budget_reason_to_eval(failure.reason),
                        instruction_count: failure.instruction_count,
                        limit: failure.limit,
                        elapsed_ms: failure.elapsed_ms,
                    }
                }
            }
        }
    }
}

impl From<skiff_runtime_capability_context::StreamRuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::StreamRuntimeError) -> Self {
        match error {
            skiff_runtime_capability_context::StreamRuntimeError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_capability_context::StreamRuntimeError::Cancelled => {
                RuntimeError::Cancelled
            }
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
                runtime_error_from_wire_payload(error)
            }
        }
    }
}

impl From<skiff_runtime_capability_context::FileCapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::FileCapabilityError) -> Self {
        match error {
            skiff_runtime_capability_context::FileCapabilityError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_capability_context::FileCapabilityError::File(message) => {
                RuntimeError::FileError { message }
            }
            skiff_runtime_capability_context::FileCapabilityError::Opaque(error) => {
                runtime_error_from_wire_payload(error)
            }
            skiff_runtime_capability_context::FileCapabilityError::ProviderUnavailable {
                target,
                reason,
            } => RuntimeError::ProviderUnavailable { target, reason },
            skiff_runtime_capability_context::FileCapabilityError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            },
            skiff_runtime_capability_context::FileCapabilityError::Stream(error) => {
                RuntimeError::from(error)
            }
            skiff_runtime_capability_context::FileCapabilityError::Execution(error) => {
                RuntimeError::from(error)
            }
        }
    }
}

impl From<skiff_runtime_capability_context::CapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::CapabilityError) -> Self {
        match error {
            skiff_runtime_capability_context::CapabilityError::Decode(message) => {
                RuntimeError::Decode(message)
            }
            skiff_runtime_capability_context::CapabilityError::Unsupported(message) => {
                RuntimeError::Unsupported(message)
            }
            skiff_runtime_capability_context::CapabilityError::ProviderUnavailable {
                target,
                reason,
            } => RuntimeError::ProviderUnavailable { target, reason },
            skiff_runtime_capability_context::CapabilityError::Protocol { target, message } => {
                RuntimeError::Protocol { target, message }
            }
            skiff_runtime_capability_context::CapabilityError::TaskSubmitRejected {
                code,
                message,
            } => RuntimeError::ProviderUnavailable {
                target: "task.submit.request".to_string(),
                reason: format!("task.submit rejected ({code}): {message}"),
            },
            skiff_runtime_capability_context::CapabilityError::TaskControlRejected {
                code,
                message,
            } => RuntimeError::ProviderUnavailable {
                target: "task.status.request".to_string(),
                reason: format!("task control rejected ({code}): {message}"),
            },
            skiff_runtime_capability_context::CapabilityError::Opaque(error) => {
                runtime_error_from_wire_payload(error)
            }
        }
    }
}

impl From<skiff_runtime_capability_context::DbCapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::DbCapabilityError) -> Self {
        match error {
            skiff_runtime_capability_context::DbCapabilityError::Decode(message) => {
                RuntimeError::DbDecode {
                    target: "std.db".to_string(),
                    message,
                }
            }
            skiff_runtime_capability_context::DbCapabilityError::ProviderUnavailable {
                target,
                reason,
            } => RuntimeError::ProviderUnavailable { target, reason },
            skiff_runtime_capability_context::DbCapabilityError::Opaque(error) => {
                runtime_error_from_wire_payload(error)
            }
        }
    }
}

impl From<skiff_runtime_capability_context::RequestPayloadContextError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::RequestPayloadContextError) -> Self {
        let message = error.to_string();
        match error {
            skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
                target,
            } => RuntimeError::Protocol { target, message },
        }
    }
}

impl From<skiff_runtime_capability_context::OutboundRequestRegistryError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::OutboundRequestRegistryError) -> Self {
        RuntimeError::Protocol {
            target: "std.service.outboundRequestRegistry".to_string(),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserException {
    request: RequestException,
    /// Decoded platform payload captured while the request heap was still
    /// alive. Without this, an unhandled builtin exception (for example
    /// `std.service.ProviderUnavailableError`) logs only its catch identity
    /// and loses the `target`/`reason` fields that make it diagnosable.
    platform_details: Option<(CatchIdentity, Value)>,
}

impl UserException {
    pub fn new(request: RequestException) -> Self {
        Self {
            request,
            platform_details: None,
        }
    }

    pub fn with_platform_details(mut self, identity: CatchIdentity, payload: Value) -> Self {
        self.platform_details = Some((identity, payload));
        self
    }

    pub fn request(&self) -> &RequestException {
        &self.request
    }

    pub fn into_request(self) -> RequestException {
        self.request
    }

    pub fn actual_payload_type(&self) -> Option<&CatchIdentity> {
        self.request.local_catch_identity()
    }

    pub fn platform_details(&self) -> Option<&(CatchIdentity, Value)> {
        self.platform_details.as_ref()
    }
}

impl fmt::Display for UserException {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.platform_details {
            Some((identity, payload)) => write!(formatter, "{identity:?} {payload}"),
            None => match self.actual_payload_type() {
                Some(identity) => write!(formatter, "{identity:?}"),
                None => formatter.write_str("opaque service error"),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequestHeapOwnedStreamError {
    inner: Arc<RequestHeapOwnedStreamErrorInner>,
}

#[derive(Debug)]
struct RequestHeapOwnedStreamErrorInner {
    error: RuntimeError,
    heap: RequestHeap,
}

impl RequestHeapOwnedStreamError {
    pub(crate) fn try_new(
        error: RuntimeError,
        heap: RequestHeap,
    ) -> std::result::Result<Self, RuntimeError> {
        if error.is_internal_execution_terminal() {
            return Err(error);
        }
        Ok(Self {
            inner: Arc::new(RequestHeapOwnedStreamErrorInner { error, heap }),
        })
    }

    fn materialize_in(&self, destination: &mut RequestHeap) -> Result<RuntimeError> {
        let cloned_error = runtime_error_from_eval_ref(&self.inner.error);
        let RuntimeError::UserException(exception) =
            unwrap_diagnostic_source_context(&self.inner.error)
        else {
            return Ok(cloned_error);
        };
        let request = exception.request().clone();
        let local_value = request.local_value().cloned();
        let request = match local_value {
            Some(value) => {
                let value = deep_clone_runtime_value_carrier_between_heaps(
                    &self.inner.heap,
                    destination,
                    &value,
                )?;
                request.map_local_value(|_| value)
            }
            None => request,
        };
        Ok(replace_user_exception_preserving_diagnostics(
            cloned_error,
            UserException::new(request),
        ))
    }
}

impl fmt::Display for RequestHeapOwnedStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner.error, formatter)
    }
}

impl std::error::Error for RequestHeapOwnedStreamError {}

impl WirePayload for RequestHeapOwnedStreamError {
    fn payload(&self) -> RuntimeErrorPayload {
        self.inner
            .error
            .ordinary_payload()
            .expect("RequestHeapOwnedStreamError construction excludes cancellation")
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.inner.error.ordinary_catch_projection()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) fn materialize_stream_runtime_error(
    error: skiff_runtime_capability_context::StreamRuntimeError,
    destination: &mut RequestHeap,
) -> Result<RuntimeError> {
    if let skiff_runtime_capability_context::StreamRuntimeError::Producer(error) = &error {
        if let Some(error) = error.as_any().downcast_ref::<RequestHeapOwnedStreamError>() {
            return error.materialize_in(destination);
        }
    }
    materialize_request_heap_owned_runtime_error(error.into(), destination)
}

pub(crate) fn materialize_request_heap_owned_runtime_error(
    error: RuntimeError,
    destination: &mut RequestHeap,
) -> Result<RuntimeError> {
    match error {
        RuntimeError::Opaque(error) => {
            if let Some(error) = error.as_any().downcast_ref::<RequestHeapOwnedStreamError>() {
                error.materialize_in(destination)
            } else {
                Ok(RuntimeError::Opaque(error))
            }
        }
        RuntimeError::WithSource {
            source_id,
            frame,
            error,
        } => Ok(RuntimeError::WithSource {
            source_id,
            frame,
            error: Box::new(materialize_request_heap_owned_runtime_error(
                *error,
                destination,
            )?),
        }),
        RuntimeError::WithDiagnosticFrame { frame, error } => {
            Ok(RuntimeError::WithDiagnosticFrame {
                frame,
                error: Box::new(materialize_request_heap_owned_runtime_error(
                    *error,
                    destination,
                )?),
            })
        }
        error => Ok(error),
    }
}

/// Moves any request-local user-exception payload from `source` into
/// `destination` while preserving the error's diagnostic wrapper structure.
/// Other errors, including cancellation and scope terminals, do not own heap
/// handles and pass through unchanged.
pub(crate) fn rematerialize_runtime_error_between_heaps(
    error: RuntimeError,
    source: &RequestHeap,
    destination: &mut RequestHeap,
) -> Result<RuntimeError> {
    let RuntimeError::UserException(exception) = unwrap_diagnostic_source_context(&error) else {
        return Ok(error);
    };
    let request = exception.request().clone();
    let Some(local_value) = request.local_value().cloned() else {
        return Ok(error);
    };
    let checkpoint = destination.checkpoint();
    let local_value =
        match deep_clone_runtime_value_carrier_between_heaps(source, destination, &local_value) {
            Ok(local_value) => local_value,
            Err(error) => {
                destination.rollback_to_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
    Ok(replace_user_exception_preserving_diagnostics(
        error,
        UserException::new(request.map_local_value(|_| local_value)),
    ))
}

pub(crate) fn runtime_error_request_heap_root(
    error: &RuntimeError,
) -> Option<&RuntimeValueCarrier> {
    let RuntimeError::UserException(exception) = unwrap_diagnostic_source_context(error) else {
        return None;
    };
    exception.request().local_value()
}

/// Rebinds only the request-heap carrier embedded in a local exception.
///
/// All typed identity, opaque-service metadata, source/diagnostic wrappers,
/// request-local stack and correlation fields remain on their original owned
/// values. This is the error-side publish step of transaction heap rollback;
/// heap graph preparation is deliberately owned by the transaction coordinator.
pub(crate) fn rebind_runtime_error_request_heap_root(
    error: RuntimeError,
    rebased: Option<skiff_runtime_model::runtime_value::RuntimeValue>,
) -> Result<RuntimeError> {
    fn rebind(
        error: RuntimeError,
        rebased: &mut Option<skiff_runtime_model::runtime_value::RuntimeValue>,
        found: &mut bool,
    ) -> RuntimeError {
        match error {
            RuntimeError::WithSource {
                source_id,
                frame,
                error,
            } => RuntimeError::WithSource {
                source_id,
                frame,
                error: Box::new(rebind(*error, rebased, found)),
            },
            RuntimeError::WithDiagnosticFrame { frame, error } => {
                RuntimeError::WithDiagnosticFrame {
                    frame,
                    error: Box::new(rebind(*error, rebased, found)),
                }
            }
            RuntimeError::UserException(exception)
                if exception.request().local_value().is_some() =>
            {
                *found = true;
                let replacement = rebased
                    .take()
                    .expect("validated transaction error root must be present");
                let request = exception
                    .into_request()
                    .map_local_value(|carrier| carrier.map_value(|_| replacement));
                RuntimeError::UserException(UserException::new(request))
            }
            error => error,
        }
    }

    let expected = runtime_error_request_heap_root(&error).is_some();
    if expected != rebased.is_some() {
        return Err(RuntimeError::InvalidArtifact(
            "transaction error rollback root mapping does not match the selected error".to_string(),
        ));
    }
    let mut rebased = rebased;
    let mut found = false;
    let error = rebind(error, &mut rebased, &mut found);
    if found != expected || rebased.is_some() {
        return Err(RuntimeError::InvalidArtifact(
            "transaction error rollback root was not consumed exactly once".to_string(),
        ));
    }
    Ok(error)
}

impl RuntimeError {
    pub fn from_wire_payload(error: Box<dyn WirePayload>) -> Self {
        runtime_error_from_wire_payload(error)
    }

    /// Returns only the strict fixed carrier, recursively ignoring local
    /// diagnostic wrappers. Generic wire payloads and message/code projections
    /// are intentionally not accepted.
    pub fn fixed_service_failure(&self) -> Option<&OpaqueServiceError> {
        match self {
            Self::FixedServiceFailure(error) => Some(error),
            Self::WithSource { error, .. } | Self::WithDiagnosticFrame { error, .. } => {
                error.fixed_service_failure()
            }
            _ => None,
        }
    }

    /// Whether this error is (or carries) an internal cancellation terminal.
    pub fn is_cancellation_terminal(&self) -> bool {
        match self {
            RuntimeError::Cancelled => true,
            RuntimeError::ExecutionBudgetExceeded { reason, .. } => {
                *reason == BudgetReason::Cancelled
            }
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => error.is_cancellation_terminal(),
            _ => false,
        }
    }

    /// Whether this error is (or carries) an eval-only execution terminal.
    pub(crate) fn is_internal_execution_terminal(&self) -> bool {
        match self {
            RuntimeError::ScopeTerminal(_) => true,
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => {
                error.is_internal_execution_terminal()
            }
            _ => self.is_cancellation_terminal(),
        }
    }

    pub(crate) fn scope_terminal(&self) -> Option<&ScopeTerminalCarrier> {
        match self {
            RuntimeError::ScopeTerminal(terminal) => Some(terminal),
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => error.scope_terminal(),
            _ => None,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancellation_terminal()
    }

    pub fn decode_target(target: impl Into<String>, message: impl Into<String>) -> Self {
        RuntimeError::DecodeTarget {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn bytes_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        RuntimeError::BytesDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn db_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        RuntimeError::DbDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn file_error(message: impl Into<String>) -> Self {
        RuntimeError::FileError {
            message: message.into(),
        }
    }

    pub fn resource_error(path: impl Into<String>, message: impl Into<String>) -> Self {
        RuntimeError::ResourceError {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn http_error(message: impl Into<String>, detail: Option<Value>) -> Self {
        RuntimeError::HttpError {
            message: message.into(),
            detail,
        }
    }

    pub fn with_source(self, source_id: u64, source_frame: Value) -> Self {
        if error_has_source_context(&self, source_id, &source_frame) {
            return self;
        }
        match self {
            RuntimeError::WithDiagnosticFrame { frame, error } => {
                RuntimeError::WithDiagnosticFrame {
                    frame,
                    error: Box::new(error.with_source(source_id, source_frame)),
                }
            }
            error => RuntimeError::WithSource {
                source_id,
                frame: Box::new(source_frame),
                error: Box::new(error),
            },
        }
    }

    pub fn with_diagnostic_frame(self, frame: Value) -> Self {
        match self {
            RuntimeError::WithDiagnosticFrame { .. } => self,
            error => RuntimeError::WithDiagnosticFrame {
                frame: Box::new(frame),
                error: Box::new(error),
            },
        }
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        Some(match self {
            RuntimeError::ActorInstance(error) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
            RuntimeError::WithSource {
                source_id,
                frame,
                error,
            } => {
                let mut payload = error.ordinary_payload()?;
                add_source_frame(&mut payload, *source_id, (**frame).clone());
                payload
            }
            RuntimeError::WithDiagnosticFrame { frame, error } => {
                let mut payload = error.ordinary_payload()?;
                add_diagnostic_frame(&mut payload, (**frame).clone());
                payload
            }
            RuntimeError::InvalidArtifact(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::DecodeTarget { target, message } => RuntimeErrorPayload {
                code: decode_target_error_code(target)
                    .unwrap_or("InternalError")
                    .to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            RuntimeError::BytesDecode { target, message } => RuntimeErrorPayload {
                code: "std.bytes.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            RuntimeError::DbDecode { target, message } => RuntimeErrorPayload {
                code: "std.db.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            RuntimeError::FileError { message } => RuntimeErrorPayload {
                code: "std.file.FileError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::ResourceError { path, message } => RuntimeErrorPayload {
                code: "std.resource.ResourceError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "path": path,
                    "message": message,
                })),
            },
            RuntimeError::HttpError { message, detail } => RuntimeErrorPayload {
                code: "std.http.HttpError".to_string(),
                message: message.clone(),
                status: None,
                details: detail.clone(),
            },
            RuntimeError::Unsupported(message) => RuntimeErrorPayload {
                code: "UnsupportedRuntimeFeature".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::Recoverable(error) => RuntimeErrorPayload {
                code: error.code().as_str().to_string(),
                message: error.message().to_string(),
                status: None,
                details: Some(error.details_json()),
            },
            RuntimeError::LeaseLost(message) => RuntimeErrorPayload {
                code: "LeaseLost".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::Cancelled => return None,
            RuntimeError::ScopeTerminal(_) => return None,
            RuntimeError::ExecutionBudgetExceeded {
                reason: BudgetReason::Cancelled,
                ..
            } => return None,
            RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => RuntimeErrorPayload {
                code: "TimeoutError".to_string(),
                message: match reason {
                    BudgetReason::DeadlineExceeded => "execution deadline exceeded".to_string(),
                    BudgetReason::InstructionLimitExceeded => {
                        "execution instruction limit exceeded".to_string()
                    }
                    BudgetReason::Cancelled => unreachable!("cancel terminal was split above"),
                },
                status: None,
                details: Some(serde_json::json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                })),
            },
            RuntimeError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeErrorPayload {
                code: "ResourceLimitExceeded".to_string(),
                message: format!("resource limit exceeded for {resource}: {reason}"),
                status: None,
                details: Some(serde_json::json!({
                    "resource": resource,
                    "reason": reason,
                    "limit": limit,
                    "current": current,
                    "requestedDelta": requested_delta,
                })),
            },
            RuntimeError::UserException(exception) => user_exception_payload(exception),
            // `RuntimeError`'s legacy diagnostic trait is total. Keep this
            // fallback constant and opaque: the canonical service channel
            // reads the strict carrier through `fixed_service_failure`.
            RuntimeError::FixedServiceFailure(_) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: "canonical service failure".to_string(),
                status: None,
                details: None,
            },
            RuntimeError::TaskDispatchFailure { target, reason } => RuntimeErrorPayload {
                code: "TaskDispatchFailure".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            RuntimeError::ProviderUnavailable { target, reason } => RuntimeErrorPayload {
                code: "std.service.ProviderUnavailableError".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            RuntimeError::Protocol { target, message } => RuntimeErrorPayload {
                code: "std.service.ProtocolError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            RuntimeError::RootRuntimePayload(payload) => payload.clone(),
            RuntimeError::Opaque(error) => error.payload(),
            RuntimeError::Json(error) => RuntimeErrorPayload {
                code: "JsonError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
        })
    }

    #[allow(dead_code)]
    pub fn diagnostic_source_id(&self) -> Option<u64> {
        self.diagnostic_source().map(|source| source.source_id)
    }

    #[allow(dead_code)]
    pub fn diagnostic_source(&self) -> Option<DiagnosticSource> {
        match self {
            RuntimeError::WithSource {
                source_id,
                frame,
                error,
            } => error
                .diagnostic_source()
                .or_else(|| diagnostic_source_from_frame(frame))
                .or(Some(DiagnosticSource {
                    assembly_id: None,
                    source_id: *source_id,
                })),
            RuntimeError::WithDiagnosticFrame { frame, error } => error
                .diagnostic_source()
                .or_else(|| diagnostic_source_from_frame(frame)),
            _ => None,
        }
    }
}

impl RuntimeError {
    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => error.ordinary_catch_projection(),
            RuntimeError::DecodeTarget { target, message } => decode_target_error_code(target)
                .and_then(|code| {
                    let identity = PlatformBuiltinErrorIdentity::from_symbol(code)?;
                    Some((
                        identity.catch_identity(),
                        serde_json::json!({
                            "target": target,
                            "message": message,
                        }),
                    ))
                }),
            RuntimeError::BytesDecode { target, message } => Some((
                PlatformBuiltinErrorIdentity::BytesDecode.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::DbDecode { target, message } => Some((
                PlatformBuiltinErrorIdentity::DbDecode.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::FileError { message } => Some((
                PlatformBuiltinErrorIdentity::File.catch_identity(),
                serde_json::json!({
                    "message": message,
                }),
            )),
            RuntimeError::HttpError { message, detail } => Some((
                PlatformBuiltinErrorIdentity::Http.catch_identity(),
                serde_json::json!({
                    "message": message,
                    "detail": detail,
                }),
            )),
            RuntimeError::Cancelled => None,
            RuntimeError::ScopeTerminal(_) => None,
            RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => {
                if *reason == BudgetReason::Cancelled {
                    return None;
                }
                Some((
                    PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
                    serde_json::json!({
                        "reason": reason.as_str(),
                        "instructionCount": instruction_count,
                        "limit": limit,
                        "elapsedMs": elapsed_ms,
                    }),
                ))
            }
            RuntimeError::ProviderUnavailable { target, reason } => Some((
                PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            RuntimeError::Protocol { target, message } => Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::Opaque(error) => error.catch_projection(),
            RuntimeError::ActorInstance(_)
            | RuntimeError::InvalidArtifact(_)
            | RuntimeError::Decode(_)
            | RuntimeError::Unsupported(_)
            | RuntimeError::Recoverable(_)
            | RuntimeError::LeaseLost(_)
            | RuntimeError::ResourceError { .. }
            | RuntimeError::ResourceLimitExceeded { .. }
            | RuntimeError::UserException(_)
            | RuntimeError::FixedServiceFailure(_)
            | RuntimeError::TaskDispatchFailure { .. }
            | RuntimeError::RootRuntimePayload(_)
            | RuntimeError::Json(_) => None,
        }
    }
}

/// Ordinary-only dynamic carrier. The constructor is the sole point where an
/// eval error may enter a total [`WirePayload`] API.
#[derive(Debug)]
pub struct OrdinaryRuntimeError(RuntimeError);

impl OrdinaryRuntimeError {
    pub fn try_new(error: RuntimeError) -> std::result::Result<Self, RuntimeError> {
        if error.is_internal_execution_terminal() {
            return Err(error);
        }
        Ok(Self(error))
    }

    pub fn error(&self) -> &RuntimeError {
        &self.0
    }
}

impl fmt::Display for OrdinaryRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for OrdinaryRuntimeError {}

impl WirePayload for OrdinaryRuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        self.0
            .ordinary_payload()
            .expect("OrdinaryRuntimeError construction excludes cancellation")
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.0.ordinary_catch_projection()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) fn stream_runtime_error_from_eval(
    error: RuntimeError,
) -> skiff_runtime_capability_context::StreamRuntimeError {
    match OrdinaryRuntimeError::try_new(error) {
        Ok(error) => skiff_runtime_capability_context::StreamRuntimeError::producer(error),
        Err(error) => {
            debug_assert!(error.is_internal_execution_terminal());
            skiff_runtime_capability_context::StreamRuntimeError::Cancelled
        }
    }
}

pub fn attach_source_frame(
    error: RuntimeError,
    source_id: u64,
    source_frame: Value,
) -> RuntimeError {
    error.with_source(source_id, source_frame)
}

pub fn attach_diagnostic_frame(error: RuntimeError, frame: Value) -> RuntimeError {
    error.with_diagnostic_frame(frame)
}

pub(crate) fn instantiated_type_argument_identity(
    argument: &impl Serialize,
) -> Result<InstantiatedTypeArgumentIdentity> {
    let canonical = serde_json::to_string(argument).map_err(RuntimeError::Json)?;
    InstantiatedTypeArgumentIdentity::new(canonical).map_err(RuntimeError::InvalidArtifact)
}

pub(crate) fn decode_opaque_service_error(bytes: Vec<u8>) -> Result<OpaqueServiceError> {
    OpaqueServiceError::decode(bytes).map_err(RuntimeError::Json)
}

/// Replaces the user-exception leaf without changing diagnostic wrapper structure.
pub(crate) fn replace_user_exception_preserving_diagnostics(
    error: RuntimeError,
    exception: UserException,
) -> RuntimeError {
    match error {
        RuntimeError::UserException(_) => RuntimeError::UserException(exception),
        RuntimeError::WithSource {
            source_id,
            frame,
            error,
        } => RuntimeError::WithSource {
            source_id,
            frame,
            error: Box::new(replace_user_exception_preserving_diagnostics(
                *error, exception,
            )),
        },
        RuntimeError::WithDiagnosticFrame { frame, error } => RuntimeError::WithDiagnosticFrame {
            frame,
            error: Box::new(replace_user_exception_preserving_diagnostics(
                *error, exception,
            )),
        },
        other => other,
    }
}

/// Extracts an actor-store leaf while preserving every diagnostic wrapper when
/// the leaf belongs to a different error class.
pub(crate) fn extract_actor_instance_store_error(
    error: RuntimeError,
) -> std::result::Result<crate::actor_instance::ActorInstanceStoreError, RuntimeError> {
    match error {
        RuntimeError::ActorInstance(error) => Ok(error),
        RuntimeError::WithSource {
            source_id,
            frame,
            error,
        } => extract_actor_instance_store_error(*error).map_err(|error| RuntimeError::WithSource {
            source_id,
            frame,
            error: Box::new(error),
        }),
        RuntimeError::WithDiagnosticFrame { frame, error } => {
            extract_actor_instance_store_error(*error).map_err(|error| {
                RuntimeError::WithDiagnosticFrame {
                    frame,
                    error: Box::new(error),
                }
            })
        }
        error => Err(error),
    }
}

pub(crate) fn diagnostic_source_frames(error: &RuntimeError) -> Vec<&Value> {
    fn collect<'a>(error: &'a RuntimeError, frames: &mut Vec<&'a Value>) {
        match error {
            RuntimeError::WithSource { frame, error, .. } => {
                frames.push(frame);
                collect(error, frames);
            }
            RuntimeError::WithDiagnosticFrame { error, .. } => collect(error, frames),
            _ => {}
        }
    }

    let mut frames = Vec::new();
    collect(error, &mut frames);
    frames
}

pub(crate) fn is_deadline_budget_terminal(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::ExecutionBudgetExceeded { reason, .. } => {
            *reason == BudgetReason::DeadlineExceeded
        }
        RuntimeError::WithSource { error, .. }
        | RuntimeError::WithDiagnosticFrame { error, .. } => is_deadline_budget_terminal(error),
        _ => false,
    }
}

pub(crate) fn is_deadline_or_scope_terminal(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::ScopeTerminal(_) => true,
        RuntimeError::WithSource { error, .. }
        | RuntimeError::WithDiagnosticFrame { error, .. } => is_deadline_or_scope_terminal(error),
        _ => is_deadline_budget_terminal(error),
    }
}

pub fn unwrap_diagnostic_source_context(error: &RuntimeError) -> &RuntimeError {
    match error {
        RuntimeError::WithSource { error, .. }
        | RuntimeError::WithDiagnosticFrame { error, .. } => {
            unwrap_diagnostic_source_context(error)
        }
        error => error,
    }
}

pub fn decode_target_error_code(target: &str) -> Option<&'static str> {
    match target {
        "std.json.decode" | "std.json.encode" | "std.resource.json" => Some("std.json.DecodeError"),
        "config.require" | "config.optional" | "config.has" => Some("config.DecodeError"),
        "number.parse" | "number.assertSafeInteger" => Some("std.number.DecodeError"),
        target if target.starts_with("Date.") || target.starts_with("Duration.") => {
            Some("std.time.DecodeError")
        }
        _ => None,
    }
}

fn user_exception_payload(exception: &UserException) -> RuntimeErrorPayload {
    let correlation = exception.request().correlation();
    let details = match exception.platform_details() {
        Some((identity, payload)) => json!({
            "traceId": correlation.trace_id,
            "errorId": correlation.error_id,
            "identity": format!("{identity:?}"),
            "payload": payload,
        }),
        None => json!({
            "traceId": correlation.trace_id,
            "errorId": correlation.error_id,
        }),
    };
    RuntimeErrorPayload {
        code: "UnhandledServiceError".to_string(),
        message: "unhandled request-local user exception".to_string(),
        status: None,
        details: Some(details),
    }
}

fn diagnostic_source_from_frame(frame: &Value) -> Option<DiagnosticSource> {
    Some(DiagnosticSource {
        assembly_id: source_assembly_id(frame),
        source_id: frame.get("sourceId").and_then(Value::as_u64)?,
    })
}

fn error_has_source_context(error: &RuntimeError, source_id: u64, frame: &Value) -> bool {
    match error {
        RuntimeError::WithSource {
            source_id: existing_source_id,
            frame: existing_frame,
            error,
        } => {
            source_context_matches(*existing_source_id, existing_frame, source_id, frame)
                || error_has_source_context(error, source_id, frame)
        }
        RuntimeError::WithDiagnosticFrame { frame: _, error } => {
            error_has_source_context(error, source_id, frame)
        }
        _ => false,
    }
}

fn source_context_matches(
    existing_source_id: u64,
    existing_frame: &Value,
    source_id: u64,
    frame: &Value,
) -> bool {
    if existing_source_id != source_id {
        return false;
    }
    let existing_assembly_id = source_assembly_id(existing_frame);
    let assembly_id = source_assembly_id(frame);
    existing_assembly_id == assembly_id || existing_assembly_id.is_none() || assembly_id.is_none()
}

fn source_assembly_id(frame: &Value) -> Option<u32> {
    frame
        .get("assemblyId")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn add_source_frame(payload: &mut RuntimeErrorPayload, source_id: u64, frame: Value) {
    let mut details = details_as_object(payload.details.take());
    details.insert("sourceId".to_string(), json!(source_id));
    details.insert("sourceFrame".to_string(), frame.clone());
    match details.get_mut("sourceFrames") {
        Some(Value::Array(frames)) => frames.insert(0, frame.clone()),
        Some(existing) => {
            let previous = existing.take();
            *existing = Value::Array(vec![frame.clone(), previous]);
        }
        None => {
            details.insert(
                "sourceFrames".to_string(),
                Value::Array(vec![frame.clone()]),
            );
        }
    }
    add_frame_to_details(&mut details, frame);
    payload.details = Some(Value::Object(details));
}

fn add_diagnostic_frame(payload: &mut RuntimeErrorPayload, frame: Value) {
    let mut details = details_as_object(payload.details.take());
    add_frame_to_details(&mut details, frame);
    payload.details = Some(Value::Object(details));
}

fn details_as_object(details: Option<Value>) -> Map<String, Value> {
    match details {
        Some(Value::Object(object)) => object,
        Some(details) => {
            let mut object = Map::new();
            object.insert("originalDetails".to_string(), details);
            object
        }
        None => Map::new(),
    }
}

fn add_frame_to_details(details: &mut Map<String, Value>, frame: Value) {
    match details.get_mut("frames") {
        Some(Value::Array(frames)) => frames.insert(0, frame),
        Some(existing) => {
            let previous = existing.take();
            *existing = Value::Array(vec![frame, previous]);
        }
        _ => {
            details.insert("frames".to_string(), Value::Array(vec![frame]));
        }
    }
}

#[cfg(test)]
mod tests;

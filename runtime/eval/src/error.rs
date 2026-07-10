use std::fmt;

use serde_json::{json, Map, Value};

pub use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

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

const EXCEPTION_MARKER_KEY: &str = "__skiffException";
const EXCEPTION_ACTUAL_PAYLOAD_TYPE_KEY: &str = "__skiffActualPayloadType";
const EXCEPTION_DECLARED_PAYLOAD_TYPE_KEY: &str = "__skiffPayloadDeclaredType";
const EXCEPTION_ACTUAL_PAYLOAD_TYPE_DEBUG_KEY: &str = "__skiffActualPayloadTypeDebug";
const EXCEPTION_DECLARED_PAYLOAD_TYPE_DEBUG_KEY: &str = "__skiffPayloadDeclaredTypeDebug";

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
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
            skiff_runtime_linked_type_plan::Error::Boundary(error) => RuntimeError::from(error),
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
/// Carriers stay `RuntimeError::Opaque` by default: their `WirePayload`
/// impls already provide the payload and catch projection, so re-projecting
/// them into eval's structured variants is unnecessary and loses the original
/// carrier. Only errors whose semantics eval (or its callers) consume
/// structurally are unpacked: eval's own errors (so `Recoverable`, budget and
/// cancellation control flow keep working after a round trip), model/native
/// control errors, and the boundary `Recoverable` variant that drives the
/// recoverable-spawn machinery.
fn runtime_error_from_wire_payload(error: Box<dyn WirePayload>) -> RuntimeError {
    if let Some(error) = error.as_any().downcast_ref::<RuntimeError>() {
        return runtime_error_from_eval_ref(error);
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
        .downcast_ref::<skiff_runtime_native::error::RuntimeError>()
    {
        return runtime_error_from_native_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return runtime_error_from_execution_control_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return runtime_error_from_stream_ref(error);
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

/// Recursion-aware cancellation detection over opaque wire carriers, mirroring
/// the host-side `wire_payload_is_request_cancelled` idiom for the error types
/// eval can observe.
fn wire_payload_is_cancelled(error: &dyn WirePayload) -> bool {
    if let Some(error) = error.as_any().downcast_ref::<RuntimeError>() {
        return error.is_cancelled();
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return match error {
            skiff_runtime_capability_context::ExecutionControlError::Cancelled => true,
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
                failure.reason == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
            }
        };
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return match error {
            skiff_runtime_capability_context::StreamRuntimeError::Cancelled => true,
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
                wire_payload_is_cancelled(error.as_ref())
            }
            skiff_runtime_capability_context::StreamRuntimeError::Decode(_) => false,
        };
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_native::error::RuntimeError>()
    {
        return match error {
            skiff_runtime_native::error::RuntimeError::Cancelled => true,
            skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
                reason, ..
            } => *reason == skiff_runtime_native::error::BudgetReason::Cancelled,
            skiff_runtime_native::error::RuntimeError::Opaque(error) => {
                wire_payload_is_cancelled(error.as_ref())
            }
            _ => false,
        };
    }
    false
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
        skiff_runtime_native::error::RuntimeError::Opaque(_) => {
            RuntimeError::Decode(error.to_string())
        }
        skiff_runtime_native::error::RuntimeError::Json(_) => {
            RuntimeError::Decode(error.to_string())
        }
    }
}

fn runtime_error_from_execution_control_ref(
    error: &skiff_runtime_capability_context::ExecutionControlError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
            RuntimeError::Cancelled
        }
        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
            if failure.reason == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
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

fn runtime_error_from_stream_ref(
    error: &skiff_runtime_capability_context::StreamRuntimeError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::StreamRuntimeError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_capability_context::StreamRuntimeError::Cancelled => RuntimeError::Cancelled,
        skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
            runtime_error_from_wire_payload_ref(error.as_ref())
        }
    }
}

fn runtime_error_from_wire_payload_ref(error: &dyn WirePayload) -> RuntimeError {
    if let Some(error) = error.as_any().downcast_ref::<RuntimeError>() {
        return runtime_error_from_eval_ref(error);
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
        .downcast_ref::<skiff_runtime_native::error::RuntimeError>()
    {
        return runtime_error_from_native_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return runtime_error_from_execution_control_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return runtime_error_from_stream_ref(error);
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
        .downcast_ref::<skiff_runtime_capability_context::FileCapabilityError>()
    {
        return runtime_error_from_file_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::RequestPayloadContextError>()
    {
        return runtime_error_from_request_payload_ref(error);
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

fn runtime_error_from_file_capability_ref(
    error: &skiff_runtime_capability_context::FileCapabilityError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::FileCapabilityError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_capability_context::FileCapabilityError::File(message) => {
            RuntimeError::FileError {
                message: message.clone(),
            }
        }
        skiff_runtime_capability_context::FileCapabilityError::Opaque(error) => {
            runtime_error_from_wire_payload_ref(error.as_ref())
        }
        skiff_runtime_capability_context::FileCapabilityError::ProviderUnavailable {
            target,
            reason,
        } => RuntimeError::ProviderUnavailable {
            target: target.clone(),
            reason: reason.clone(),
        },
        skiff_runtime_capability_context::FileCapabilityError::ResourceLimitExceeded {
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
        skiff_runtime_capability_context::FileCapabilityError::Stream(error) => {
            runtime_error_from_stream_ref(error)
        }
        skiff_runtime_capability_context::FileCapabilityError::Execution(error) => {
            runtime_error_from_execution_control_ref(error)
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
        error => skiff_runtime_native::error::RuntimeError::Opaque(Box::new(error)),
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
            // Cancellation is a control signal, not a domain error: keep the
            // wire carrier opaque so payload/catch projections delegate to the
            // control error itself and cancellation detection recurses through
            // the carrier (see `RuntimeError::is_cancelled`).
            skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
                RuntimeError::Opaque(Box::new(error))
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

#[derive(Debug, Clone)]
pub struct UserException {
    actual_payload_type: TypeIdentity,
    envelope: Value,
}

impl UserException {
    pub fn from_payload(
        payload: Value,
        payload_declared_type: Option<TypeIdentity>,
    ) -> Result<Self> {
        let actual_payload_type = payload_declared_type.clone().ok_or_else(|| {
            RuntimeError::Decode(
                "throw payload requires an explicit static payload type".to_string(),
            )
        })?;
        Self::from_typed_payload(payload, actual_payload_type, payload_declared_type)
    }

    pub fn from_typed_payload(
        payload: Value,
        actual_payload_type: TypeIdentity,
        payload_declared_type: Option<TypeIdentity>,
    ) -> Result<Self> {
        let mut envelope = Map::new();
        envelope.insert(EXCEPTION_MARKER_KEY.to_string(), Value::Bool(true));
        envelope.insert(
            EXCEPTION_ACTUAL_PAYLOAD_TYPE_KEY.to_string(),
            serde_json::to_value(&actual_payload_type)?,
        );
        envelope.insert(
            EXCEPTION_ACTUAL_PAYLOAD_TYPE_DEBUG_KEY.to_string(),
            Value::String(actual_payload_type.to_string()),
        );
        if let Some(payload_declared_type) = payload_declared_type {
            envelope.insert(
                EXCEPTION_DECLARED_PAYLOAD_TYPE_KEY.to_string(),
                serde_json::to_value(&payload_declared_type)?,
            );
            envelope.insert(
                EXCEPTION_DECLARED_PAYLOAD_TYPE_DEBUG_KEY.to_string(),
                Value::String(payload_declared_type.to_string()),
            );
        }
        envelope.insert("error".to_string(), payload);
        envelope.insert("source".to_string(), Value::Null);
        envelope.insert("stack".to_string(), Value::Array(Vec::new()));

        Ok(Self {
            actual_payload_type,
            envelope: Value::Object(envelope),
        })
    }

    pub fn from_envelope(envelope: Value) -> Result<Self> {
        let object = envelope.as_object().ok_or_else(|| {
            RuntimeError::Decode("rethrow operand must be an exception object".to_string())
        })?;
        if object.get(EXCEPTION_MARKER_KEY).and_then(Value::as_bool) != Some(true) {
            return Err(RuntimeError::Decode(
                "rethrow operand must be a request-local exception envelope".to_string(),
            ));
        }
        let actual_payload_type_value =
            object
                .get(EXCEPTION_ACTUAL_PAYLOAD_TYPE_KEY)
                .ok_or_else(|| {
                    RuntimeError::Decode(
                        "rethrow exception missing actual payload type".to_string(),
                    )
                })?;
        let actual_payload_type = serde_json::from_value::<TypeIdentity>(
            actual_payload_type_value.clone(),
        )
        .map_err(|error| {
            RuntimeError::Decode(format!(
                "rethrow exception has invalid actual payload type: {error}"
            ))
        })?;
        if !object.contains_key("error") {
            return Err(RuntimeError::Decode(
                "rethrow exception missing error payload".to_string(),
            ));
        }

        Ok(Self {
            actual_payload_type,
            envelope,
        })
    }

    pub fn from_runtime_parts(actual_payload_type: TypeIdentity, envelope: Value) -> Self {
        Self {
            actual_payload_type,
            envelope,
        }
    }

    pub fn actual_payload_type(&self) -> &TypeIdentity {
        &self.actual_payload_type
    }

    pub fn error_payload(&self) -> Option<&Map<String, Value>> {
        self.envelope
            .as_object()
            .and_then(|object| object.get("error"))
            .and_then(Value::as_object)
    }

    pub fn envelope(&self) -> Value {
        self.envelope.clone()
    }
}

impl fmt::Display for UserException {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.actual_payload_type)
    }
}

impl RuntimeError {
    pub fn from_wire_payload(error: Box<dyn WirePayload>) -> Self {
        runtime_error_from_wire_payload(error)
    }

    /// Whether this error is (or carries) a cancellation signal.
    ///
    /// Cancellation may arrive either as eval's structured
    /// `RuntimeError::Cancelled` or as an opaque wire carrier (e.g.
    /// `ExecutionControlError::Cancelled` boxed by
    /// `From<ExecutionControlError>` or by the host root). Interpreter sites
    /// that swallow or forward cancellation must use this recursion-aware
    /// check instead of matching `RuntimeError::Cancelled` structurally.
    pub fn is_cancelled(&self) -> bool {
        match self {
            RuntimeError::Cancelled => true,
            RuntimeError::ExecutionBudgetExceeded { reason, .. } => {
                *reason == BudgetReason::Cancelled
            }
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => error.is_cancelled(),
            RuntimeError::Opaque(error) => wire_payload_is_cancelled(error.as_ref()),
            _ => false,
        }
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

    #[allow(dead_code)]
    pub fn payload(&self) -> RuntimeErrorPayload {
        match self {
            RuntimeError::WithSource {
                source_id,
                frame,
                error,
            } => {
                let mut payload = error.payload();
                add_source_frame(&mut payload, *source_id, (**frame).clone());
                payload
            }
            RuntimeError::WithDiagnosticFrame { frame, error } => {
                let mut payload = error.payload();
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
            RuntimeError::Cancelled => RuntimeErrorPayload {
                code: "CancelError".to_string(),
                message: "request was cancelled".to_string(),
                status: None,
                details: None,
            },
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
                    BudgetReason::Cancelled => "request was cancelled".to_string(),
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
        }
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

impl WirePayload for RuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeError::payload(self)
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        match self {
            RuntimeError::WithSource { error, .. }
            | RuntimeError::WithDiagnosticFrame { error, .. } => error.catch_projection(),
            RuntimeError::DecodeTarget { target, message } => {
                decode_target_error_code(target).map(|code| {
                    (
                        TypeIdentity::builtin(code),
                        serde_json::json!({
                            "target": target,
                            "message": message,
                        }),
                    )
                })
            }
            RuntimeError::BytesDecode { target, message } => Some((
                TypeIdentity::builtin("std.bytes.DecodeError"),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::DbDecode { target, message } => Some((
                TypeIdentity::builtin("std.db.DecodeError"),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::FileError { message } => Some((
                TypeIdentity::builtin("std.file.FileError"),
                serde_json::json!({
                    "message": message,
                }),
            )),
            RuntimeError::ResourceError { path, message } => Some((
                TypeIdentity::builtin("std.resource.ResourceError"),
                serde_json::json!({
                    "path": path,
                    "message": message,
                }),
            )),
            RuntimeError::HttpError { message, detail } => Some((
                TypeIdentity::builtin("std.http.HttpError"),
                serde_json::json!({
                    "message": message,
                    "detail": detail,
                }),
            )),
            RuntimeError::Cancelled => Some((
                TypeIdentity::builtin("CancelError"),
                serde_json::json!({
                    "message": "request was cancelled",
                }),
            )),
            RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => Some((
                TypeIdentity::builtin("TimeoutError"),
                serde_json::json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                }),
            )),
            RuntimeError::ProviderUnavailable { target, reason } => Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                serde_json::json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            RuntimeError::Protocol { target, message } => Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::Opaque(error) => error.catch_projection(),
            RuntimeError::InvalidArtifact(_)
            | RuntimeError::Decode(_)
            | RuntimeError::Unsupported(_)
            | RuntimeError::Recoverable(_)
            | RuntimeError::LeaseLost(_)
            | RuntimeError::ResourceLimitExceeded { .. }
            | RuntimeError::UserException(_)
            | RuntimeError::RootRuntimePayload(_)
            | RuntimeError::Json(_) => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        "std.json.decode" | "std.json.encode" | "std.resource.json" => {
            Some("std.json.DecodeError")
        }
        "config.require" | "config.optional" | "config.has" => Some("config.DecodeError"),
        "number.parse" | "number.assertSafeInteger" => Some("std.number.DecodeError"),
        target if target.starts_with("Date.") || target.starts_with("Duration.") => {
            Some("std.time.DecodeError")
        }
        _ => None,
    }
}

fn user_exception_payload(exception: &UserException) -> RuntimeErrorPayload {
    let actual_payload_type = exception.actual_payload_type().to_string();
    let message = exception
        .error_payload()
        .and_then(|payload| payload.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(|message| format!("unhandled user exception {actual_payload_type}: {message}"))
        .unwrap_or_else(|| format!("unhandled user exception {actual_payload_type}"));
    RuntimeErrorPayload {
        code: "UnhandledServiceError".to_string(),
        message,
        status: None,
        details: Some(serde_json::json!({
            "actualPayloadType": actual_payload_type,
        })),
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
mod tests {
    use skiff_runtime_boundary::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode};
    use skiff_runtime_model::recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    };

    use super::*;

    fn recoverable_boundary_error() -> RecoverableBoundaryError {
        let context = RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot();
        let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");

        RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::UnsupportedDecode,
            "recoverable decode is unsupported",
            &context,
            &expected,
        )
    }

    #[test]
    fn recoverable_payload_uses_boundary_details_contract() {
        let error = recoverable_boundary_error();
        let expected_details = error.details_json();

        let payload = RuntimeError::Recoverable(error).payload();

        assert_eq!(payload.code, "recoverableUnsupportedDecode");
        assert_eq!(payload.status, None);
        assert_eq!(payload.details, Some(expected_details));
    }

    #[test]
    fn wire_payload_delegates_to_inherent_payload_with_default_catch_projection() {
        let error = RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code: "DownstreamError".to_string(),
            message: "downstream failed".to_string(),
            status: Some(503),
            details: Some(serde_json::json!({ "service": "account" })),
        })
        .with_diagnostic_frame(serde_json::json!({ "sourceId": 7 }));

        assert_eq!(WirePayload::payload(&error), error.payload());
        assert_eq!(WirePayload::catch_projection(&error), None);
        assert!(WirePayload::as_any(&error).is::<RuntimeError>());
    }

    fn assert_catch_projection(
        error: RuntimeError,
        expected_identity: &'static str,
        expected_payload: Value,
    ) {
        assert_eq!(
            WirePayload::catch_projection(&error),
            Some((TypeIdentity::builtin(expected_identity), expected_payload))
        );
    }

    #[test]
    fn catch_projection_covers_standard_eval_errors() {
        assert_catch_projection(
            RuntimeError::DecodeTarget {
                target: "std.json.decode".to_string(),
                message: "invalid json".to_string(),
            },
            "std.json.DecodeError",
            serde_json::json!({
                "target": "std.json.decode",
                "message": "invalid json",
            }),
        );
        assert_catch_projection(
            RuntimeError::BytesDecode {
                target: "bytes.toUtf8String".to_string(),
                message: "invalid utf8".to_string(),
            },
            "std.bytes.DecodeError",
            serde_json::json!({
                "target": "bytes.toUtf8String",
                "message": "invalid utf8",
            }),
        );
        assert_catch_projection(
            RuntimeError::DbDecode {
                target: "std.db".to_string(),
                message: "missing id".to_string(),
            },
            "std.db.DecodeError",
            serde_json::json!({
                "target": "std.db",
                "message": "missing id",
            }),
        );
        assert_catch_projection(
            RuntimeError::FileError {
                message: "std.file not found".to_string(),
            },
            "std.file.FileError",
            serde_json::json!({
                "message": "std.file not found",
            }),
        );
        assert_catch_projection(
            RuntimeError::resource_error("prompts/system.md", "missing"),
            "std.resource.ResourceError",
            serde_json::json!({
                "path": "prompts/system.md",
                "message": "missing",
            }),
        );
        assert_catch_projection(
            RuntimeError::HttpError {
                message: "std.http.request failed".to_string(),
                detail: Some(serde_json::json!({ "status": 500 })),
            },
            "std.http.HttpError",
            serde_json::json!({
                "message": "std.http.request failed",
                "detail": { "status": 500 },
            }),
        );
        assert_catch_projection(
            RuntimeError::Cancelled,
            "CancelError",
            serde_json::json!({
                "message": "request was cancelled",
            }),
        );
        assert_catch_projection(
            RuntimeError::ExecutionBudgetExceeded {
                reason: BudgetReason::InstructionLimitExceeded,
                instruction_count: 42,
                limit: Some(100),
                elapsed_ms: 12.5,
            },
            "TimeoutError",
            serde_json::json!({
                "reason": "instructionLimitExceeded",
                "instructionCount": 42,
                "limit": 100,
                "elapsedMs": 12.5,
            }),
        );
        assert_catch_projection(
            RuntimeError::ProviderUnavailable {
                target: "svc.account".to_string(),
                reason: "no runtime".to_string(),
            },
            "std.service.ProviderUnavailableError",
            serde_json::json!({
                "target": "svc.account",
                "reason": "no runtime",
            }),
        );
        assert_catch_projection(
            RuntimeError::Protocol {
                target: "svc.account".to_string(),
                message: "bad frame".to_string(),
            },
            "std.service.ProtocolError",
            serde_json::json!({
                "target": "svc.account",
                "message": "bad frame",
            }),
        );
    }

    #[test]
    fn unknown_decode_target_is_not_catchable() {
        let error = RuntimeError::DecodeTarget {
            target: "runtime.config".to_string(),
            message: "path apiKey must be a string".to_string(),
        };

        assert_eq!(WirePayload::catch_projection(&error), None);
    }

    #[test]
    fn root_runtime_payload_is_wire_only() {
        let stored = RuntimeErrorPayload {
            code: "DownstreamError".to_string(),
            message: "downstream failed".to_string(),
            status: Some(503),
            details: Some(serde_json::json!({ "service": "account" })),
        };
        let error = RuntimeError::RootRuntimePayload(stored.clone());

        assert_eq!(error.payload(), stored);
        assert_eq!(WirePayload::catch_projection(&error), None);
    }

    #[test]
    fn domain_forward_preserves_concrete_error_variants_payload_and_catch_projection() {
        let model_error = skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
            resource: "request.heap".to_string(),
            reason: "too large".to_string(),
            limit: 10,
            current: 8,
            requested_delta: 4,
        };
        let expected_model_payload = model_error.payload();
        let error = RuntimeError::from(model_error);
        assert!(matches!(
            error,
            RuntimeError::ResourceLimitExceeded {
                ref resource,
                ..
            } if resource == "request.heap"
        ));
        assert_eq!(error.payload(), expected_model_payload);
        assert_eq!(WirePayload::catch_projection(&error), None);

        let boundary_error =
            skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied");
        let expected_boundary_payload = boundary_error.payload();
        let expected_boundary_catch_projection = boundary_error.catch_projection();
        let error = RuntimeError::from(boundary_error);
        assert!(matches!(error, RuntimeError::FileError { .. }));
        assert_eq!(error.payload(), expected_boundary_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_boundary_catch_projection
        );

        let linked_error = skiff_runtime_linked_type_plan::Error::Protocol {
            target: "svc.account".to_string(),
            message: "bad payload".to_string(),
        };
        let expected_linked_payload = linked_error.payload();
        let expected_linked_catch_projection = linked_error.catch_projection();
        let error = RuntimeError::from(linked_error);
        assert!(matches!(error, RuntimeError::Protocol { .. }));
        assert_eq!(error.payload(), expected_linked_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_linked_catch_projection
        );
    }

    #[test]
    fn eval_to_native_back_projection_preserves_concrete_non_control_errors() {
        let error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::DbDecode {
            target: "std.db".to_string(),
            message: "missing id".to_string(),
        });

        assert!(matches!(error, RuntimeError::DbDecode { .. }));
        let native = eval_error_to_native(error);
        assert!(matches!(
            native,
            skiff_runtime_native::error::RuntimeError::DbDecode { ref target, ref message }
                if target == "std.db" && message == "missing id"
        ));
    }

    #[test]
    fn eval_to_native_back_projection_preserves_diagnostic_wrappers_as_opaque() {
        let source_frame = serde_json::json!({ "sourceId": 7 });
        let diagnostic_frame = serde_json::json!({ "operation": "eval.test" });
        let error = RuntimeError::FileError {
            message: "std.file denied".to_string(),
        }
        .with_source(7, source_frame.clone())
        .with_diagnostic_frame(diagnostic_frame.clone());

        let native = eval_error_to_native(error);

        match native {
            skiff_runtime_native::error::RuntimeError::Opaque(error) => {
                let payload = error.payload();
                assert_eq!(payload.code, "std.file.FileError");
                let details = payload.details.expect("diagnostic details should exist");
                assert_eq!(details["sourceId"].as_u64(), Some(7));
                assert_eq!(details["frames"][0], diagnostic_frame);
                assert_eq!(details["frames"][1], source_frame);
                assert_eq!(
                    error.catch_projection(),
                    Some((
                        TypeIdentity::builtin("std.file.FileError"),
                        serde_json::json!({ "message": "std.file denied" }),
                    ))
                );
            }
            error => panic!("expected native Opaque, got {error:?}"),
        }
    }

    #[test]
    fn capability_context_errors_preserve_concrete_variants_payload_and_catch_projection() {
        let file_error =
            skiff_runtime_capability_context::FileCapabilityError::file("std.file not found");
        let expected_payload = file_error.payload();
        let expected_catch_projection = file_error.catch_projection();
        let error = RuntimeError::from(file_error);
        assert!(matches!(error, RuntimeError::FileError { .. }));
        assert_eq!(error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_catch_projection
        );

        let protocol_error =
            skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
                target: "svc.account".to_string(),
            };
        let expected_payload = protocol_error.payload();
        let expected_catch_projection = protocol_error.catch_projection();
        let error = RuntimeError::from(protocol_error);
        assert!(matches!(
            error,
            RuntimeError::Protocol { ref target, .. } if target == "svc.account"
        ));
        assert_eq!(error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_catch_projection
        );

        let timeout_error = skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
            skiff_runtime_capability_context::ExecutionBudgetFailure {
                reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
                instruction_count: 42,
                limit: Some(100),
                elapsed_ms: 12.5,
            },
        );
        let expected_payload = timeout_error.payload();
        let expected_catch_projection = timeout_error.catch_projection();
        let error = RuntimeError::from(timeout_error);
        assert!(matches!(
            error,
            RuntimeError::ExecutionBudgetExceeded {
                reason: BudgetReason::DeadlineExceeded,
                ..
            }
        ));
        assert_eq!(error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_catch_projection
        );
    }
}

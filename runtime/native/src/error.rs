use serde_json::json;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetReason {
    Cancelled,
    DeadlineExceeded,
    InstructionLimitExceeded,
}

impl BudgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "deadlineExceeded",
            Self::InstructionLimitExceeded => "instructionLimitExceeded",
        }
    }
}

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
    #[error("http error: {message}")]
    HttpError {
        message: String,
        detail: Option<serde_json::Value>,
    },
    #[error("{0}")]
    Unsupported(String),
    #[error(transparent)]
    Recoverable(#[from] skiff_runtime_boundary::error::RecoverableBoundaryError),
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
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
    #[error("{0}")]
    Opaque(Box<dyn WirePayload>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl RuntimeError {
    pub fn decode_target(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DecodeTarget {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn bytes_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::BytesDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn db_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DbDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn file_error(message: impl Into<String>) -> Self {
        Self::FileError {
            message: message.into(),
        }
    }

    pub fn http_error(message: impl Into<String>, detail: Option<serde_json::Value>) -> Self {
        Self::HttpError {
            message: message.into(),
            detail,
        }
    }
}

impl WirePayload for RuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::InvalidArtifact(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::DecodeTarget { target, message } => RuntimeErrorPayload {
                code: skiff_runtime_boundary::error::decode_target_error_code(target)
                    .unwrap_or("InternalError")
                    .to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::BytesDecode { target, message } => RuntimeErrorPayload {
                code: "std.bytes.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::DbDecode { target, message } => RuntimeErrorPayload {
                code: "std.db.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::FileError { message } => RuntimeErrorPayload {
                code: "std.file.FileError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::HttpError { message, detail } => RuntimeErrorPayload {
                code: "std.http.HttpError".to_string(),
                message: message.clone(),
                status: None,
                details: detail.clone(),
            },
            Self::Unsupported(message) => RuntimeErrorPayload {
                code: "UnsupportedRuntimeFeature".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Recoverable(error) => RuntimeErrorPayload {
                code: error.code().as_str().to_string(),
                message: error.message().to_string(),
                status: None,
                details: Some(error.details_json()),
            },
            Self::Cancelled => RuntimeErrorPayload {
                code: "CancelError".to_string(),
                message: "request was cancelled".to_string(),
                status: None,
                details: None,
            },
            Self::ExecutionBudgetExceeded {
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
                details: Some(json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                })),
            },
            Self::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeErrorPayload {
                code: "ResourceLimitExceeded".to_string(),
                message: format!("resource limit exceeded for {resource}: {reason}"),
                status: None,
                details: Some(json!({
                    "resource": resource,
                    "reason": reason,
                    "limit": limit,
                    "current": current,
                    "requestedDelta": requested_delta,
                })),
            },
            Self::Opaque(error) => error.payload(),
            Self::Json(error) => RuntimeErrorPayload {
                code: "JsonError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            Self::DecodeTarget { target, message } => {
                skiff_runtime_boundary::error::decode_target_error_code(target).map(|code| {
                    (
                        TypeIdentity::builtin(code),
                        json!({
                            "target": target,
                            "message": message,
                        }),
                    )
                })
            }
            Self::BytesDecode { target, message } => Some((
                TypeIdentity::builtin("std.bytes.DecodeError"),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::DbDecode { target, message } => Some((
                TypeIdentity::builtin("std.db.DecodeError"),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::FileError { message } => Some((
                TypeIdentity::builtin("std.file.FileError"),
                json!({
                    "message": message,
                }),
            )),
            Self::HttpError { message, detail } => Some((
                TypeIdentity::builtin("std.http.HttpError"),
                json!({
                    "message": message,
                    "detail": detail,
                }),
            )),
            Self::Cancelled => Some((
                TypeIdentity::builtin("CancelError"),
                json!({
                    "message": "request was cancelled",
                }),
            )),
            Self::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => Some((
                TypeIdentity::builtin("TimeoutError"),
                json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                }),
            )),
            Self::InvalidArtifact(_)
            | Self::Decode(_)
            | Self::Unsupported(_)
            | Self::Recoverable(_)
            | Self::ResourceLimitExceeded { .. }
            | Self::Json(_) => None,
            Self::Opaque(error) => error.catch_projection(),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl From<skiff_runtime_boundary::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_boundary::RuntimeError) -> Self {
        match error {
            skiff_runtime_boundary::RuntimeError::InvalidArtifact(message) => {
                RuntimeError::InvalidArtifact(message)
            }
            skiff_runtime_boundary::RuntimeError::Decode(message) => RuntimeError::Decode(message),
            skiff_runtime_boundary::RuntimeError::DecodeTarget { target, message } => {
                RuntimeError::DecodeTarget { target, message }
            }
            skiff_runtime_boundary::RuntimeError::BytesDecode { target, message } => {
                RuntimeError::BytesDecode { target, message }
            }
            skiff_runtime_boundary::RuntimeError::DbDecode { target, message } => {
                RuntimeError::DbDecode { target, message }
            }
            skiff_runtime_boundary::RuntimeError::FileError { message } => {
                RuntimeError::FileError { message }
            }
            skiff_runtime_boundary::RuntimeError::HttpError { message, detail } => {
                RuntimeError::HttpError { message, detail }
            }
            skiff_runtime_boundary::RuntimeError::Unsupported(message) => {
                RuntimeError::Unsupported(message)
            }
            skiff_runtime_boundary::RuntimeError::Recoverable(error) => {
                RuntimeError::Recoverable(error)
            }
            skiff_runtime_boundary::RuntimeError::ResourceLimitExceeded {
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
            skiff_runtime_boundary::RuntimeError::Json(error) => RuntimeError::Json(error),
        }
    }
}

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
                native_error_from_wire_payload(error)
            }
            skiff_runtime_capability_context::FileCapabilityError::ProviderUnavailable {
                target,
                reason,
            } => RuntimeError::Unsupported(format!("provider unavailable for {target}: {reason}")),
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
                native_error_from_wire_payload(error)
            }
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
                        reason: capability_budget_reason_to_native(failure.reason),
                        instruction_count: failure.instruction_count,
                        limit: failure.limit,
                        elapsed_ms: failure.elapsed_ms,
                    }
                }
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
            } => RuntimeError::Unsupported(format!("provider unavailable for {target}: {reason}")),
            skiff_runtime_capability_context::CapabilityError::Protocol { target, message } => {
                RuntimeError::DecodeTarget { target, message }
            }
            skiff_runtime_capability_context::CapabilityError::Opaque(error) => {
                native_error_from_wire_payload(error)
            }
        }
    }
}

fn capability_budget_reason_to_native(
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

fn native_error_from_wire_payload(error: Box<dyn WirePayload>) -> RuntimeError {
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return native_error_from_model_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::RuntimeError>()
    {
        return native_error_from_boundary_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return native_error_from_execution_control_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return native_error_from_stream_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::FileCapabilityError>()
    {
        return native_error_from_file_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::CapabilityError>()
    {
        return native_error_from_capability_ref(error);
    }
    RuntimeError::Opaque(error)
}

fn native_error_from_wire_payload_ref(error: &dyn WirePayload) -> RuntimeError {
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return native_error_from_model_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::RuntimeError>()
    {
        return native_error_from_boundary_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::ExecutionControlError>()
    {
        return native_error_from_execution_control_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::StreamRuntimeError>()
    {
        return native_error_from_stream_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::FileCapabilityError>()
    {
        return native_error_from_file_capability_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::CapabilityError>()
    {
        return native_error_from_capability_ref(error);
    }
    RuntimeError::Decode(error.to_string())
}

fn native_error_from_model_ref(
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

fn native_error_from_boundary_ref(error: &skiff_runtime_boundary::RuntimeError) -> RuntimeError {
    match error {
        skiff_runtime_boundary::RuntimeError::InvalidArtifact(message) => {
            RuntimeError::InvalidArtifact(message.clone())
        }
        skiff_runtime_boundary::RuntimeError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_boundary::RuntimeError::DecodeTarget { target, message } => {
            RuntimeError::DecodeTarget {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::RuntimeError::BytesDecode { target, message } => {
            RuntimeError::BytesDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::RuntimeError::DbDecode { target, message } => {
            RuntimeError::DbDecode {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_boundary::RuntimeError::FileError { message } => RuntimeError::FileError {
            message: message.clone(),
        },
        skiff_runtime_boundary::RuntimeError::HttpError { message, detail } => {
            RuntimeError::HttpError {
                message: message.clone(),
                detail: detail.clone(),
            }
        }
        skiff_runtime_boundary::RuntimeError::Unsupported(message) => {
            RuntimeError::Unsupported(message.clone())
        }
        skiff_runtime_boundary::RuntimeError::Recoverable(error) => {
            RuntimeError::Recoverable(error.clone())
        }
        skiff_runtime_boundary::RuntimeError::ResourceLimitExceeded {
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
        skiff_runtime_boundary::RuntimeError::Json(_) => RuntimeError::Decode(error.to_string()),
    }
}

fn native_error_from_execution_control_ref(
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
                    reason: capability_budget_reason_to_native(failure.reason),
                    instruction_count: failure.instruction_count,
                    limit: failure.limit,
                    elapsed_ms: failure.elapsed_ms,
                }
            }
        }
    }
}

fn native_error_from_stream_ref(
    error: &skiff_runtime_capability_context::StreamRuntimeError,
) -> RuntimeError {
    match error {
        skiff_runtime_capability_context::StreamRuntimeError::Decode(message) => {
            RuntimeError::Decode(message.clone())
        }
        skiff_runtime_capability_context::StreamRuntimeError::Cancelled => RuntimeError::Cancelled,
        skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
            native_error_from_wire_payload_ref(error.as_ref())
        }
    }
}

fn native_error_from_file_capability_ref(
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
            native_error_from_wire_payload_ref(error.as_ref())
        }
        skiff_runtime_capability_context::FileCapabilityError::ProviderUnavailable {
            target,
            reason,
        } => RuntimeError::Unsupported(format!("provider unavailable for {target}: {reason}")),
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
            native_error_from_stream_ref(error)
        }
        skiff_runtime_capability_context::FileCapabilityError::Execution(error) => {
            native_error_from_execution_control_ref(error)
        }
    }
}

fn native_error_from_capability_ref(
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
        } => RuntimeError::Unsupported(format!("provider unavailable for {target}: {reason}")),
        skiff_runtime_capability_context::CapabilityError::Protocol { target, message } => {
            RuntimeError::DecodeTarget {
                target: target.clone(),
                message: message.clone(),
            }
        }
        skiff_runtime_capability_context::CapabilityError::Opaque(error) => {
            native_error_from_wire_payload_ref(error.as_ref())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct DummyWirePayload;

    impl fmt::Display for DummyWirePayload {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("dummy wire payload")
        }
    }

    impl std::error::Error for DummyWirePayload {}

    impl WirePayload for DummyWirePayload {
        fn payload(&self) -> RuntimeErrorPayload {
            RuntimeErrorPayload {
                code: "test.NativeOpaque".to_string(),
                message: "dummy wire payload".to_string(),
                status: Some(499),
                details: Some(serde_json::json!({ "nativeOpaque": true })),
            }
        }

        fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
            Some((
                TypeIdentity::builtin("test.NativeOpaqueCatch"),
                serde_json::json!({ "caught": true }),
            ))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn budget_reason_strings_are_public_wire_values() {
        assert_eq!(BudgetReason::Cancelled.as_str(), "cancelled");
        assert_eq!(BudgetReason::DeadlineExceeded.as_str(), "deadlineExceeded");
        assert_eq!(
            BudgetReason::InstructionLimitExceeded.as_str(),
            "instructionLimitExceeded"
        );
    }

    #[test]
    fn native_payload_covers_boundary_like_and_request_control_variants() {
        let decode_target = RuntimeError::decode_target("number.parse", "not a number").payload();
        assert_eq!(decode_target.code, "std.number.DecodeError");
        assert_eq!(
            decode_target.details,
            Some(serde_json::json!({
                "target": "number.parse",
                "message": "not a number",
            }))
        );

        let cancelled = RuntimeError::Cancelled.payload();
        assert_eq!(cancelled.code, "CancelError");
        assert_eq!(cancelled.message, "request was cancelled");

        let timeout = RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        }
        .payload();
        assert_eq!(timeout.code, "TimeoutError");
        assert_eq!(timeout.message, "execution deadline exceeded");
        assert_eq!(
            timeout.details,
            Some(serde_json::json!({
                "reason": "deadlineExceeded",
                "instructionCount": 42,
                "limit": 100,
                "elapsedMs": 12.5,
            }))
        );
    }

    #[test]
    fn native_catch_projection_covers_public_catchable_variants() {
        assert_eq!(
            RuntimeError::decode_target("Date.requireParse", "bad date").catch_projection(),
            Some((
                TypeIdentity::builtin("std.time.DecodeError"),
                serde_json::json!({
                    "target": "Date.requireParse",
                    "message": "bad date",
                })
            ))
        );
        assert_eq!(
            RuntimeError::file_error("std.file denied").catch_projection(),
            Some((
                TypeIdentity::builtin("std.file.FileError"),
                serde_json::json!({
                    "message": "std.file denied",
                })
            ))
        );
        assert_eq!(
            RuntimeError::Cancelled.catch_projection(),
            Some((
                TypeIdentity::builtin("CancelError"),
                serde_json::json!({
                    "message": "request was cancelled",
                })
            ))
        );
        assert_eq!(
            RuntimeError::ExecutionBudgetExceeded {
                reason: BudgetReason::InstructionLimitExceeded,
                instruction_count: 42,
                limit: Some(100),
                elapsed_ms: 12.5,
            }
            .catch_projection(),
            Some((
                TypeIdentity::builtin("TimeoutError"),
                serde_json::json!({
                    "reason": "instructionLimitExceeded",
                    "instructionCount": 42,
                    "limit": 100,
                    "elapsedMs": 12.5,
                })
            ))
        );
    }

    #[test]
    fn native_opaque_delegates_payload_and_catch_projection() {
        let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

        assert_eq!(error.payload().code, "test.NativeOpaque");
        assert_eq!(error.payload().status, Some(499));
        assert_eq!(
            error.catch_projection(),
            Some((
                TypeIdentity::builtin("test.NativeOpaqueCatch"),
                serde_json::json!({ "caught": true }),
            ))
        );
    }
}

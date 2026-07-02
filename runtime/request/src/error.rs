use serde_json::Value;
use skiff_runtime_capability_context::{
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError, ResponseError,
};
use skiff_runtime_eval::error::RuntimeError as EvalRuntimeError;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

pub type RequestResult<T> = std::result::Result<T, RequestError>;

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("protocol error for {target}: {message}")]
    Protocol { target: String, message: String },
    #[error("request was cancelled")]
    Cancelled,
    #[error("execution budget exceeded: {reason:?}")]
    ExecutionBudgetExceeded {
        reason: ExecutionBudgetReason,
        instruction_count: u64,
        limit: Option<u64>,
        elapsed_ms: f64,
    },
    #[error(transparent)]
    Eval(#[from] EvalRuntimeError),
    #[error(transparent)]
    Boundary(#[from] skiff_runtime_boundary::error::RuntimeError),
    #[error("{message}")]
    ExternalErrorPayload {
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    },
}

impl RequestError {
    pub fn protocol(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Protocol {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn external_error_payload(
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    ) -> Self {
        Self::ExternalErrorPayload {
            code,
            message,
            status,
            details,
        }
    }

    pub fn response_error(&self) -> ResponseError {
        let payload = self.payload();
        ResponseError {
            code: payload.code,
            message: payload.message,
            status: payload.status,
            details: payload.details,
        }
    }
}

impl WirePayload for RequestError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Unsupported(message) => RuntimeErrorPayload {
                code: "UnsupportedRuntimeFeature".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Protocol { target, message } => RuntimeErrorPayload {
                code: "std.service.ProtocolError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
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
                    ExecutionBudgetReason::DeadlineExceeded => {
                        "execution deadline exceeded".to_string()
                    }
                    ExecutionBudgetReason::InstructionLimitExceeded => {
                        "execution instruction limit exceeded".to_string()
                    }
                    ExecutionBudgetReason::Cancelled => "request was cancelled".to_string(),
                },
                status: None,
                details: Some(serde_json::json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                })),
            },
            Self::Eval(error) => error.payload(),
            Self::Boundary(error) => error.payload(),
            Self::ExternalErrorPayload {
                code,
                message,
                status,
                details,
            } => RuntimeErrorPayload {
                code: code.clone(),
                message: message.clone(),
                status: *status,
                details: details.clone(),
            },
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        match self {
            Self::Protocol { target, message } => Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::Cancelled => Some((
                TypeIdentity::builtin("CancelError"),
                serde_json::json!({
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
                serde_json::json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                }),
            )),
            Self::Eval(error) => error.catch_projection(),
            Self::Boundary(error) => error.catch_projection(),
            Self::Decode(_) | Self::Unsupported(_) | Self::ExternalErrorPayload { .. } => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl From<ExecutionControlError> for RequestError {
    fn from(error: ExecutionControlError) -> Self {
        match error {
            ExecutionControlError::Cancelled => Self::Cancelled,
            ExecutionControlError::BudgetExceeded(failure) => Self::from(failure),
        }
    }
}

impl From<ExecutionBudgetFailure> for RequestError {
    fn from(failure: ExecutionBudgetFailure) -> Self {
        if failure.reason == ExecutionBudgetReason::Cancelled {
            return Self::Cancelled;
        }
        Self::ExecutionBudgetExceeded {
            reason: failure.reason,
            instruction_count: failure.instruction_count,
            limit: failure.limit,
            elapsed_ms: failure.elapsed_ms,
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
    fn boundary_recoverable_response_uses_boundary_details_contract() {
        let error = recoverable_boundary_error();
        let expected_details = error.details_json();

        let response = RequestError::Boundary(
            skiff_runtime_boundary::error::RuntimeError::Recoverable(error),
        )
        .response_error();

        assert_eq!(response.code, "recoverableUnsupportedDecode");
        assert_eq!(response.status, None);
        assert_eq!(response.details, Some(expected_details));
    }

    #[test]
    fn response_error_is_derived_from_wire_payload() {
        let error = RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        };

        let payload = error.payload();
        let response = error.response_error();

        assert_eq!(response.code, payload.code);
        assert_eq!(response.message, payload.message);
        assert_eq!(response.status, payload.status);
        assert_eq!(response.details, payload.details);
    }

    #[test]
    fn request_wire_payload_preserves_external_payload_shape() {
        let error = RequestError::external_error_payload(
            "DownstreamError".to_string(),
            "downstream failed".to_string(),
            Some(503),
            Some(serde_json::json!({ "service": "account" })),
        );

        let payload = error.payload();

        assert_eq!(payload.code, "DownstreamError");
        assert_eq!(payload.message, "downstream failed");
        assert_eq!(payload.status, Some(503));
        assert_eq!(
            payload.details,
            Some(serde_json::json!({ "service": "account" }))
        );
        assert_eq!(error.catch_projection(), None);
    }

    #[test]
    fn request_wire_payload_delegates_boundary_projection() {
        let boundary = skiff_runtime_boundary::error::RuntimeError::http_error(
            "std.http failed",
            Some(serde_json::json!({ "status": 500 })),
        );
        let expected_payload = boundary.payload();
        let expected_catch_projection = boundary.catch_projection();
        let error = RequestError::Boundary(boundary);

        assert_eq!(error.payload(), expected_payload);
        assert_eq!(error.catch_projection(), expected_catch_projection);
    }

    #[test]
    fn request_catch_projection_covers_protocol_and_request_control_errors() {
        assert_eq!(
            RequestError::protocol("svc.account", "bad frame").catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                serde_json::json!({
                    "target": "svc.account",
                    "message": "bad frame",
                })
            ))
        );
        assert_eq!(
            RequestError::Cancelled.catch_projection(),
            Some((
                TypeIdentity::builtin("CancelError"),
                serde_json::json!({
                    "message": "request was cancelled",
                })
            ))
        );
        assert_eq!(
            RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::InstructionLimitExceeded,
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
}

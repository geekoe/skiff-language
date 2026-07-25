use serde_json::Value;
use skiff_runtime_capability_context::{
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError,
    FixedServiceResponseFailure, ResponseError,
};
use skiff_runtime_eval::error::RuntimeError as EvalRuntimeError;
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, OpaqueServiceError, PlatformBuiltinErrorIdentity},
};

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

    /// Returns only the strict fixed service carrier held by eval.
    ///
    /// Generic response metadata is intentionally not inspected, so a control
    /// error with matching code/message values can never be upgraded to fixed.
    pub fn fixed_service_failure(&self) -> Option<&OpaqueServiceError> {
        match self {
            Self::Eval(error) => error.fixed_service_failure(),
            _ => None,
        }
    }

    pub fn fixed_service_response_failure(&self) -> Option<FixedServiceResponseFailure> {
        self.fixed_service_failure()
            .cloned()
            .map(FixedServiceResponseFailure::new)
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

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::Protocol { target, message } => Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::Cancelled => Some((
                PlatformBuiltinErrorIdentity::Cancel.catch_identity(),
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
                PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
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
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": "svc.account",
                    "message": "bad frame",
                })
            ))
        );
        assert_eq!(
            RequestError::Cancelled.catch_projection(),
            Some((
                PlatformBuiltinErrorIdentity::Cancel.catch_identity(),
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
                PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
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
    fn fixed_service_failure_is_extracted_only_from_the_typed_eval_carrier() {
        let encoded = br#"{
          "kind":"internalError",
          "payload":{
            "message":"The service could not complete the request.",
            "traceId":"trace-request-fixed",
            "errorId":"error-request-fixed"
          }
        }"#
        .to_vec();
        let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
        let error = RequestError::Eval(EvalRuntimeError::WithDiagnosticFrame {
            frame: Box::new(serde_json::json!({
                "message": "provider-private-secret",
            })),
            error: Box::new(EvalRuntimeError::FixedServiceFailure(fixed)),
        });

        let extracted = error
            .fixed_service_failure()
            .expect("typed fixed carrier must remain available to request");
        assert_eq!(extracted.encoded_bytes(), encoded);

        let generic = RequestError::external_error_payload(
            "InternalError".to_string(),
            "canonical service failure".to_string(),
            Some(500),
            None,
        );
        assert!(
            generic.fixed_service_failure().is_none(),
            "matching generic control values must not be upgraded"
        );
    }

    #[test]
    fn fixed_service_response_failure_preserves_all_envelope_bytes() {
        let fixtures = [
            br#"{"kind":"publicTypedError","packageId":"example.com/errors","stableSchemaKey":"not-found","packageSchemaTypeId":"type:not-found","encodedPayload":[123,125],"traceId":"trace-public","errorId":"error-public"}"#
                .as_slice(),
            br#"{
              "kind":"internalError",
              "payload":{
                "message":"The service could not complete the request.",
                "traceId":"trace-internal",
                "errorId":"error-internal"
              }
            }"#
            .as_slice(),
            br#"{"kind":"platformError","builtinErrorIdentity":"std.db.ConflictError","encodedPayload":[123,125],"traceId":"trace-platform","errorId":"error-platform"}"#
                .as_slice(),
        ];

        for encoded in fixtures {
            let encoded = encoded.to_vec();
            let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
            let error = RequestError::Eval(EvalRuntimeError::FixedServiceFailure(fixed));

            let response = error
                .fixed_service_response_failure()
                .expect("typed response failure");
            assert_eq!(response.error().encoded_bytes(), encoded);
        }
    }
}

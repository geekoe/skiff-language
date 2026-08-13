use serde_json::Value;
use skiff_runtime_capability_context::{
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError,
    FixedServiceResponseFailure, ResponseError,
};
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, OpaqueServiceError, PlatformBuiltinErrorIdentity},
};

pub type RequestResult<T> = std::result::Result<T, RequestError>;

/// Request cancellation is an internal completion terminal, not a wire error.
///
/// ```compile_fail
/// use skiff_runtime_model::error::WirePayload;
/// use skiff_runtime_request::RequestError;
///
/// let _ = WirePayload::payload(&RequestError::Cancelled);
/// ```
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

    pub fn is_cancellation_terminal(&self) -> bool {
        match self {
            Self::Cancelled => true,
            Self::ExecutionBudgetExceeded { reason, .. } => reason.is_cancellation_terminal(),
            Self::Decode(_)
            | Self::Unsupported(_)
            | Self::Protocol { .. }
            | Self::Boundary(_)
            | Self::ExternalErrorPayload { .. } => false,
        }
    }

    pub fn ordinary_response_error(&self) -> Option<ResponseError> {
        let payload = self.ordinary_payload()?;
        Some(ResponseError {
            code: payload.code,
            message: payload.message,
            status: payload.status,
            details: payload.details,
        })
    }

    pub fn fixed_service_failure(&self) -> Option<&OpaqueServiceError> {
        None
    }

    pub fn fixed_service_response_failure(&self) -> Option<FixedServiceResponseFailure> {
        self.fixed_service_failure()
            .cloned()
            .map(FixedServiceResponseFailure::new)
    }
    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        Some(match self {
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
            Self::Cancelled => return None,
            Self::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => {
                if reason.is_cancellation_terminal() {
                    return None;
                }
                RuntimeErrorPayload {
                    code: match reason {
                        ExecutionBudgetReason::DeadlineExceeded => "TimeoutError".to_string(),
                        ExecutionBudgetReason::InstructionLimitExceeded => {
                            "std.error.InstructionLimitExceededError".to_string()
                        }
                        ExecutionBudgetReason::Cancelled => {
                            unreachable!("cancellation terminal was split above")
                        }
                    },
                    message: match reason {
                        ExecutionBudgetReason::DeadlineExceeded => {
                            "execution deadline exceeded".to_string()
                        }
                        ExecutionBudgetReason::InstructionLimitExceeded => {
                            "execution instruction limit exceeded".to_string()
                        }
                        ExecutionBudgetReason::Cancelled => {
                            unreachable!("cancellation terminal was split above")
                        }
                    },
                    status: None,
                    details: Some(match reason {
                        ExecutionBudgetReason::DeadlineExceeded => serde_json::json!({
                            "reason": reason.as_str(),
                            "instructionCount": instruction_count,
                            "limit": limit,
                            "elapsedMs": elapsed_ms,
                        }),
                        ExecutionBudgetReason::InstructionLimitExceeded => serde_json::json!({
                            "instructionCount": instruction_count,
                            "limit": limit,
                        }),
                        ExecutionBudgetReason::Cancelled => {
                            unreachable!("cancellation terminal was split above")
                        }
                    }),
                }
            }
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
        })
    }

    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::Protocol { target, message } => Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::Cancelled => None,
            Self::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => {
                if reason.is_cancellation_terminal() {
                    None
                } else {
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
            }
            Self::Boundary(error) => error.catch_projection(),
            Self::Decode(_) | Self::Unsupported(_) | Self::ExternalErrorPayload { .. } => None,
        }
    }
}

impl skiff_runtime_capability_context::OrdinaryResponseErrorSource for RequestError {
    fn ordinary_response_error(&self) -> Option<ResponseError> {
        RequestError::ordinary_response_error(self)
    }
}

/// Ordinary-only request carrier for APIs that still require [`WirePayload`].
#[derive(Debug)]
pub struct OrdinaryRequestError(RequestError);

impl OrdinaryRequestError {
    pub fn try_new(error: RequestError) -> std::result::Result<Self, RequestError> {
        if error.is_cancellation_terminal() {
            return Err(error);
        }
        Ok(Self(error))
    }

    pub fn error(&self) -> &RequestError {
        &self.0
    }
}

impl std::fmt::Display for OrdinaryRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for OrdinaryRequestError {}

impl WirePayload for OrdinaryRequestError {
    fn payload(&self) -> RuntimeErrorPayload {
        self.0
            .ordinary_payload()
            .expect("OrdinaryRequestError construction excludes cancellation")
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.0.ordinary_catch_projection()
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
mod tests;

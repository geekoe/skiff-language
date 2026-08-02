use skiff_artifact_model::InstructionSourceSite;

use crate::value::RuntimeValueCarrier;

pub use skiff_runtime_request_contract::{
    CatchIdentity, InstantiatedTypeArgumentIdentity, InternalErrorPayload, LiteralIdentity,
    LocalExecutionTypeIdentity, NamedUnionBranchIdentity, NamedUnionOwnerIdentity,
    NominalTypeIdentity, OpaqueServiceError, PackageSchemaTypeIdentity,
    PlatformBuiltinErrorIdentity, ServiceErrorEnvelope, WebSocketRequestError,
    WebSocketRequestErrorKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCorrelation {
    pub trace_id: String,
    pub error_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionStackFrame {
    Local {
        site: InstructionSourceSite,
    },
    RemoteBoundary {
        service_id: String,
        operation_id: String,
        error_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestExceptionCause {
    Local {
        value: RuntimeValueCarrier,
    },
    /// An imported service failure. `local_value` is present only when the
    /// fixed error was materialized into an exact caller-local value.
    OpaqueService {
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
    },
}

/// Request-local exception state. It is deliberately not serializable.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestException {
    cause: RequestExceptionCause,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
}

impl RequestException {
    pub fn local(
        value: RuntimeValueCarrier,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
        correlation: ErrorCorrelation,
    ) -> Result<Self, String> {
        if value.catch_identity().is_none() {
            return Err("local exception value requires an actual catch identity".to_string());
        }
        if stack.is_empty() {
            return Err("local exception requires a non-empty request-local stack".to_string());
        }
        validate_correlation(&correlation.trace_id, &correlation.error_id)?;
        Ok(Self {
            cause: RequestExceptionCause::Local { value },
            source,
            stack,
            correlation,
        })
    }

    pub fn imported(
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
    ) -> Result<Self, String> {
        if let Some(value) = &local_value {
            if value.catch_identity().is_none() {
                return Err(
                    "imported local exception value requires an actual catch identity".to_string(),
                );
            }
        }
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        Ok(Self {
            cause: RequestExceptionCause::OpaqueService { error, local_value },
            source,
            stack,
            correlation,
        })
    }

    pub fn cause(&self) -> &RequestExceptionCause {
        &self.cause
    }

    pub fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub fn stack(&self) -> &[ExceptionStackFrame] {
        &self.stack
    }

    pub fn correlation(&self) -> &ErrorCorrelation {
        &self.correlation
    }

    pub fn local_catch_identity(&self) -> Option<&CatchIdentity> {
        self.local_value()
            .and_then(RuntimeValueCarrier::catch_identity)
    }

    pub fn local_value(&self) -> Option<&RuntimeValueCarrier> {
        match &self.cause {
            RequestExceptionCause::Local { value } => Some(value),
            RequestExceptionCause::OpaqueService { local_value, .. } => local_value.as_ref(),
        }
    }

    pub fn fixed_service_error(&self) -> Option<&OpaqueServiceError> {
        match &self.cause {
            RequestExceptionCause::Local { .. } => None,
            RequestExceptionCause::OpaqueService { error, .. } => Some(error),
        }
    }

    pub fn map_local_value(
        self,
        map: impl FnOnce(RuntimeValueCarrier) -> RuntimeValueCarrier,
    ) -> Self {
        let cause = match self.cause {
            RequestExceptionCause::Local { value } => {
                RequestExceptionCause::Local { value: map(value) }
            }
            RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(value),
            } => RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(map(value)),
            },
            imported @ RequestExceptionCause::OpaqueService {
                local_value: None, ..
            } => imported,
        };
        Self { cause, ..self }
    }
}

fn non_empty_ref(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(format!(
            "{label} must not be empty or contain surrounding whitespace"
        ));
    }
    Ok(())
}

fn validate_correlation(trace_id: &str, error_id: &str) -> Result<(), String> {
    non_empty_ref("traceId", trace_id)?;
    non_empty_ref("errorId", error_id)
}

#[cfg(test)]
mod tests;

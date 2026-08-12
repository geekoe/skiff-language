use std::borrow::Cow;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeModelError {
    #[error("{0}")]
    Decode(String),
    #[error("resource limit exceeded for {resource}: {reason}")]
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RuntimeModelError>;

pub use skiff_runtime_request_contract::{
    DiagnosticAttributeRecordOutcome, DiagnosticAttributes, DiagnosticCode, DiagnosticFieldKey,
    DiagnosticFieldValue, RuntimeDiagnostic, RuntimeErrorPayload, StaticDiagnosticToken,
    WirePayload, MAX_DIAGNOSTIC_ATTRIBUTES,
};

const INTERNAL_ERROR_CODE: DiagnosticCode = match DiagnosticCode::new("InternalError") {
    Some(code) => code,
    None => panic!("InternalError must be a valid diagnostic code"),
};
const RESOURCE_LIMIT_EXCEEDED_CODE: DiagnosticCode =
    match DiagnosticCode::new("ResourceLimitExceeded") {
        Some(code) => code,
        None => panic!("ResourceLimitExceeded must be a valid diagnostic code"),
    };
const JSON_ERROR_CODE: DiagnosticCode = match DiagnosticCode::new("JsonError") {
    Some(code) => code,
    None => panic!("JsonError must be a valid diagnostic code"),
};

const LIMIT_FIELD: DiagnosticFieldKey = match DiagnosticFieldKey::new("limit") {
    Some(key) => key,
    None => panic!("limit must be a valid diagnostic field key"),
};
const CURRENT_FIELD: DiagnosticFieldKey = match DiagnosticFieldKey::new("current") {
    Some(key) => key,
    None => panic!("current must be a valid diagnostic field key"),
};
const REQUESTED_DELTA_FIELD: DiagnosticFieldKey = match DiagnosticFieldKey::new("requested_delta") {
    Some(key) => key,
    None => panic!("requested_delta must be a valid diagnostic field key"),
};

impl RuntimeDiagnostic for RuntimeModelError {
    fn diagnostic_code(&self) -> DiagnosticCode {
        match self {
            Self::Decode(_) => INTERNAL_ERROR_CODE,
            Self::ResourceLimitExceeded { .. } => RESOURCE_LIMIT_EXCEEDED_CODE,
            Self::Json(_) => JSON_ERROR_CODE,
        }
    }

    fn diagnostic_message(&self) -> Cow<'_, str> {
        match self {
            Self::Decode(message) => Cow::Borrowed(message.as_str()),
            Self::ResourceLimitExceeded {
                resource, reason, ..
            } => Cow::Owned(format!("resource limit exceeded for {resource}: {reason}")),
            Self::Json(error) => Cow::Owned(error.to_string()),
        }
    }

    fn record_diagnostic_attributes(&self, attributes: &mut DiagnosticAttributes) {
        if let Self::ResourceLimitExceeded {
            limit,
            current,
            requested_delta,
            ..
        } = self
        {
            let _ = attributes.record(LIMIT_FIELD, DiagnosticFieldValue::U64(*limit as u64));
            let _ = attributes.record(CURRENT_FIELD, DiagnosticFieldValue::U64(*current as u64));
            let _ = attributes.record(
                REQUESTED_DELTA_FIELD,
                DiagnosticFieldValue::U64(*requested_delta as u64),
            );
        }
    }
}

impl skiff_runtime_request_contract::WirePayload for RuntimeModelError {
    fn payload(&self) -> skiff_runtime_request_contract::RuntimeErrorPayload {
        match self {
            Self::Decode(message) => skiff_runtime_request_contract::RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => skiff_runtime_request_contract::RuntimeErrorPayload {
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
            Self::Json(error) => skiff_runtime_request_contract::RuntimeErrorPayload {
                code: "JsonError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;

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

pub use skiff_runtime_request_contract::{RuntimeErrorPayload, WirePayload};

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

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::addr::TypeAddr;

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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl fmt::Display for RuntimeErrorPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TypeIdentity {
    Address { addr: TypeAddr },
    Builtin { name: String },
}

impl TypeIdentity {
    pub fn address(addr: TypeAddr) -> Self {
        Self::Address { addr }
    }

    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin { name: name.into() }
    }
}

impl fmt::Display for TypeIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address { addr } => write!(formatter, "{addr}"),
            Self::Builtin { name } => write!(formatter, "{name}"),
        }
    }
}

pub trait WirePayload: std::error::Error + Send + Sync + 'static {
    fn payload(&self) -> RuntimeErrorPayload;

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any;
}

impl WirePayload for RuntimeModelError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) => RuntimeErrorPayload {
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
            Self::Json(error) => RuntimeErrorPayload {
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
mod tests {
    use super::*;

    #[test]
    fn model_error_payload_covers_domain_variants() {
        let decode = RuntimeModelError::Decode("invalid runtime type".to_string()).payload();
        assert_eq!(decode.code, "InternalError");
        assert_eq!(decode.message, "invalid runtime type");
        assert_eq!(decode.details, None);

        let limit = RuntimeModelError::ResourceLimitExceeded {
            resource: "heap".to_string(),
            reason: "request heap limit exceeded".to_string(),
            limit: 1024,
            current: 900,
            requested_delta: 200,
        }
        .payload();
        assert_eq!(limit.code, "ResourceLimitExceeded");
        assert_eq!(
            limit.details,
            Some(serde_json::json!({
                "resource": "heap",
                "reason": "request heap limit exceeded",
                "limit": 1024,
                "current": 900,
                "requestedDelta": 200,
            }))
        );

        let json_error: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
        let json = RuntimeModelError::Json(json_error).payload();
        assert_eq!(json.code, "JsonError");
        assert_eq!(json.details, None);
    }

    #[test]
    fn model_error_is_not_catchable() {
        let error = RuntimeModelError::Decode("invalid runtime type".to_string());

        assert_eq!(WirePayload::catch_projection(&error), None);
        assert!(WirePayload::as_any(&error).is::<RuntimeModelError>());
    }
}

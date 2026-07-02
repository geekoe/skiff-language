use std::{error::Error, fmt, future::Future, pin::Pin};

use serde_json::{json, Value};
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

#[derive(Debug)]
pub enum CapabilityError {
    Decode(String),
    Unsupported(String),
    ProviderUnavailable { target: String, reason: String },
    Protocol { target: String, message: String },
    Opaque(Box<dyn WirePayload>),
}

impl CapabilityError {
    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    pub fn provider_unavailable(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ProviderUnavailable {
            target: target.into(),
            reason: reason.into(),
        }
    }

    pub fn protocol(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Protocol {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn opaque(error: impl WirePayload) -> Self {
        Self::Opaque(Box::new(error))
    }
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(message) | Self::Unsupported(message) => formatter.write_str(message),
            Self::ProviderUnavailable { target, reason } => {
                write!(formatter, "provider unavailable for {target}: {reason}")
            }
            Self::Protocol { target, message } => {
                write!(formatter, "protocol error for {target}: {message}")
            }
            Self::Opaque(error) => error.fmt(formatter),
        }
    }
}

impl Error for CapabilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Opaque(error) => Some(error.as_ref()),
            Self::Decode(_)
            | Self::Unsupported(_)
            | Self::ProviderUnavailable { .. }
            | Self::Protocol { .. } => None,
        }
    }
}

impl WirePayload for CapabilityError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) | Self::Unsupported(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::ProviderUnavailable { target, reason } => RuntimeErrorPayload {
                code: "std.service.ProviderUnavailableError".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            Self::Protocol { target, message } => RuntimeErrorPayload {
                code: "std.service.ProtocolError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::Opaque(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        match self {
            Self::ProviderUnavailable { target, reason } => Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            Self::Protocol { target, message } => Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::Opaque(error) => error.catch_projection(),
            Self::Decode(_) | Self::Unsupported(_) => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub type CapabilityResult<T> = Result<T, CapabilityError>;
pub type CapabilityFuture<'a, T> =
    Pin<Box<dyn Future<Output = CapabilityResult<T>> + Send + 'a>>;

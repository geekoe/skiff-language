use serde_json::json;
use skiff_runtime_boundary::error::RuntimeError as BoundaryRuntimeError;
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidArtifact(String),
    #[error("protocol error for {target}: {message}")]
    Protocol { target: String, message: String },
    #[error(transparent)]
    Boundary(#[from] Box<BoundaryRuntimeError>),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<BoundaryRuntimeError> for Error {
    fn from(error: BoundaryRuntimeError) -> Self {
        Self::Boundary(Box::new(error))
    }
}

impl WirePayload for Error {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::InvalidArtifact(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: message.clone(),
                status: None,
                details: None,
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
            Self::Boundary(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
        match self {
            Self::Protocol { target, message } => Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::Boundary(error) => error.catch_projection(),
            Self::InvalidArtifact(_) => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;

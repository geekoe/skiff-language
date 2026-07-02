use serde_json::json;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidArtifact(String),
    #[error("protocol error for {target}: {message}")]
    Protocol { target: String, message: String },
    #[error(transparent)]
    Boundary(#[from] skiff_runtime_boundary::error::RuntimeError),
}

pub type Result<T> = std::result::Result<T, Error>;

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

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            Self::Protocol { target, message } => Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
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
mod tests {
    use super::*;

    #[test]
    fn linked_type_plan_protocol_payload_and_catch_projection_are_service_protocol() {
        let error = Error::Protocol {
            target: "svc.account".to_string(),
            message: "bad request payload".to_string(),
        };

        let payload = error.payload();
        assert_eq!(payload.code, "std.service.ProtocolError");
        assert_eq!(
            payload.details,
            Some(json!({
                "target": "svc.account",
                "message": "bad request payload",
            }))
        );
        assert_eq!(
            error.catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                json!({
                    "target": "svc.account",
                    "message": "bad request payload",
                })
            ))
        );
    }

    #[test]
    fn linked_type_plan_boundary_delegates_payload_and_catch_projection() {
        let boundary = skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied");
        let expected_payload = boundary.payload();
        let expected_catch_projection = boundary.catch_projection();
        let error = Error::Boundary(boundary);

        assert_eq!(error.payload(), expected_payload);
        assert_eq!(error.catch_projection(), expected_catch_projection);
    }
}

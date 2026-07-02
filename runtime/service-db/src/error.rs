use serde_json::json;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

#[derive(Debug, thiserror::Error)]
pub enum ServiceDbError {
    #[error("{0}")]
    InvalidDbMetadata(String),
    #[error("{0}")]
    Decode(String),
    #[error("db decode error for {target}: {message}")]
    DbDecode { target: String, message: String },
    #[error("db lease lost: {0}")]
    LeaseLost(String),
    #[error(transparent)]
    Mongo(#[from] mongodb::error::Error),
    #[error(transparent)]
    BsonSer(#[from] mongodb::bson::ser::Error),
    #[error(transparent)]
    BsonDe(#[from] mongodb::bson::de::Error),
    #[error("{0}")]
    Opaque(Box<dyn WirePayload>),
}

pub type Result<T> = std::result::Result<T, ServiceDbError>;

impl ServiceDbError {
    pub fn db_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DbDecode {
            target: target.into(),
            message: message.into(),
        }
    }
}

impl WirePayload for ServiceDbError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            ServiceDbError::InvalidDbMetadata(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            ServiceDbError::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            ServiceDbError::DbDecode { target, message } => RuntimeErrorPayload {
                code: "std.db.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            ServiceDbError::LeaseLost(message) => RuntimeErrorPayload {
                code: "LeaseLost".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            ServiceDbError::Mongo(error) => RuntimeErrorPayload {
                code: "PlatformMongoError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
            ServiceDbError::BsonSer(error) => RuntimeErrorPayload {
                code: "PlatformBsonEncodeError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
            ServiceDbError::BsonDe(error) => RuntimeErrorPayload {
                code: "PlatformBsonDecodeError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
            ServiceDbError::Opaque(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            ServiceDbError::Opaque(error) => error.catch_projection(),
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        match self {
            ServiceDbError::Opaque(error) => error.as_any(),
            _ => self,
        }
    }
}

impl From<skiff_runtime_model::error::RuntimeModelError> for ServiceDbError {
    fn from(error: skiff_runtime_model::error::RuntimeModelError) -> Self {
        ServiceDbError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_boundary::error::RuntimeError> for ServiceDbError {
    fn from(error: skiff_runtime_boundary::error::RuntimeError) -> Self {
        ServiceDbError::Opaque(Box::new(error))
    }
}

use serde_json::json;
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};

use crate::mongo::{is_mongo_db_conflict_error, is_mongo_duplicate_key_error};

const DB_CONFLICT_TARGET: &str = "std.db";
const DB_CONFLICT_MESSAGE: &str =
    "database conflict; retry only at an explicit side-effect-safe boundary";
const DB_CONSTRAINT_MESSAGE: &str = "database constraint rejected the write";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbConstraintKind {
    Unique,
}

impl DbConstraintKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unique => "unique",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbConstraintTarget {
    package_id: String,
    collection: String,
}

impl DbConstraintTarget {
    pub fn new(package_id: impl Into<String>, collection: impl Into<String>) -> Result<Self> {
        let package_id = package_id.into();
        if package_id.trim().is_empty() || package_id != package_id.trim() {
            return Err(ServiceDbError::InvalidDbMetadata(
                "database constraint packageId must not be empty".to_string(),
            ));
        }
        let collection = collection.into();
        if collection.trim().is_empty() || collection != collection.trim() {
            return Err(ServiceDbError::InvalidDbMetadata(
                "database constraint logical collection identity must not be empty".to_string(),
            ));
        }
        Ok(Self {
            package_id,
            collection,
        })
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbConstraintViolation {
    kind: DbConstraintKind,
    target: DbConstraintTarget,
}

impl DbConstraintViolation {
    pub fn unique(target: DbConstraintTarget) -> Self {
        Self {
            kind: DbConstraintKind::Unique,
            target,
        }
    }

    pub const fn kind(&self) -> DbConstraintKind {
        self.kind
    }

    pub fn target(&self) -> &DbConstraintTarget {
        &self.target
    }

    fn details(&self) -> serde_json::Value {
        json!({
            "kind": self.kind.as_str(),
            "packageId": self.target.package_id(),
            "collection": self.target.collection(),
        })
    }
}

fn db_conflict_details() -> serde_json::Value {
    json!({
        "target": DB_CONFLICT_TARGET,
        "message": DB_CONFLICT_MESSAGE,
        "retryable": true,
    })
}

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
    #[error("database constraint rejected the write")]
    Constraint(DbConstraintViolation),
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

    /// Reclassifies a Mongo write failure using only the logical DB identity
    /// already trusted by the runtime program. Backend diagnostics are never
    /// copied into the public constraint payload.
    pub fn classify_write_constraint(self, target: &DbConstraintTarget) -> Self {
        match self {
            Self::Mongo(error) if is_mongo_duplicate_key_error(&error) => {
                Self::Constraint(DbConstraintViolation::unique(target.clone()))
            }
            error => error,
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
            ServiceDbError::Constraint(violation) => RuntimeErrorPayload {
                code: "std.db.ConstraintError".to_string(),
                message: DB_CONSTRAINT_MESSAGE.to_string(),
                status: None,
                details: Some(violation.details()),
            },
            ServiceDbError::Mongo(error) if is_mongo_db_conflict_error(error) => {
                RuntimeErrorPayload {
                    code: "std.db.ConflictError".to_string(),
                    message: DB_CONFLICT_MESSAGE.to_string(),
                    status: None,
                    details: Some(db_conflict_details()),
                }
            }
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

    fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
        match self {
            ServiceDbError::Constraint(violation) => Some((
                PlatformBuiltinErrorIdentity::DbConstraint.catch_identity(),
                violation.details(),
            )),
            ServiceDbError::Mongo(error) if is_mongo_db_conflict_error(error) => Some((
                PlatformBuiltinErrorIdentity::DbConflict.catch_identity(),
                db_conflict_details(),
            )),
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

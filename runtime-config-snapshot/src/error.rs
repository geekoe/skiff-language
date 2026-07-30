use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeConfigSnapshotError {
    #[error("{operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid runtime config snapshot at {path}: {message}")]
    InvalidRecord { path: PathBuf, message: String },
    #[error("runtime config snapshot already exists at {path}")]
    AlreadyExists { path: PathBuf },
    #[error("runtime config snapshot does not exist at {path}")]
    NotFound { path: PathBuf },
    #[error("runtime config snapshot JSON failed at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub type RuntimeConfigSnapshotResult<T> = Result<T, RuntimeConfigSnapshotError>;

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: std::io::Error,
) -> RuntimeConfigSnapshotError {
    RuntimeConfigSnapshotError::Io {
        operation,
        path: path.into(),
        source,
    }
}

pub(crate) fn invalid(
    path: impl Into<PathBuf>,
    message: impl Into<String>,
) -> RuntimeConfigSnapshotError {
    RuntimeConfigSnapshotError::InvalidRecord {
        path: path.into(),
        message: message.into(),
    }
}

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EcosystemStorageError {
    #[error(transparent)]
    Artifact(#[from] skiff_artifact_identity::ArtifactIdentityError),
    #[error("{operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("strict JSON parse failed for {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("immutable record conflict at {path}")]
    ImmutableConflict { path: PathBuf },
    #[error("invalid canonical ecosystem record at {path}: {message}")]
    InvalidRecord { path: PathBuf, message: String },
    #[error("compare-and-swap failed for {path}: {message}")]
    CasMismatch { path: PathBuf, message: String },
}

pub type StorageResult<T> = Result<T, EcosystemStorageError>;

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: std::io::Error,
) -> EcosystemStorageError {
    EcosystemStorageError::Io {
        operation,
        path: path.into(),
        source,
    }
}

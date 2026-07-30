use std::{io, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigSnapshotToolingError {
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path}: invalid YAML: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("{path}: invalid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("runtime config snapshot: {0}")]
    Snapshot(#[from] skiff_runtime_config_snapshot::RuntimeConfigSnapshotError),
}

pub type ConfigSnapshotToolingResult<T> = Result<T, ConfigSnapshotToolingError>;

pub(crate) fn invalid(
    path: impl Into<PathBuf>,
    message: impl Into<String>,
) -> ConfigSnapshotToolingError {
    ConfigSnapshotToolingError::Invalid {
        path: path.into(),
        message: message.into(),
    }
}

pub(crate) fn io_error(
    action: &'static str,
    path: impl Into<PathBuf>,
    source: io::Error,
) -> ConfigSnapshotToolingError {
    ConfigSnapshotToolingError::Io {
        action,
        path: path.into(),
        source,
    }
}

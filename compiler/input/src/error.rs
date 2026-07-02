use thiserror::Error;

use crate::package_config::PackageConfigError;

#[derive(Debug, Error)]
pub enum InputAssemblyError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("input validation failed:\n{message}")]
    Validation { message: String },
    #[error("service id {service_id} is invalid: {message}")]
    InvalidServiceId { service_id: String, message: String },
    #[error("{source}")]
    PackageConfig {
        #[from]
        source: PackageConfigError,
    },
}

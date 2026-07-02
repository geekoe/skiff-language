use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublicationError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: skiff_syntax::error::CompileError,
    },
    #[error("service publication exports no interfaces")]
    NoExportedInterfaces,
    #[error("contract validation failed:\n{message}")]
    ContractValidation { message: String },
    #[error("service implementation conformance failed:\n{message}")]
    ImplementationConformance { message: String },
    #[error("service id {service_id} is invalid: {message}")]
    InvalidServiceId { service_id: String, message: String },
    #[error("{source}")]
    PackageConfig {
        #[from]
        source: crate::input::PackageConfigError,
    },
    #[error("invalid root reference in {path}:\n{message}")]
    RootPathReference { path: String, message: String },
}

impl From<skiff_compiler_input::InputAssemblyError> for PublicationError {
    fn from(error: skiff_compiler_input::InputAssemblyError) -> Self {
        use skiff_compiler_input::InputAssemblyError as E;

        match error {
            E::Read { path, source } => Self::Read { path, source },
            E::Validation { message } => Self::ContractValidation { message },
            E::InvalidServiceId {
                service_id,
                message,
            } => Self::InvalidServiceId {
                service_id,
                message,
            },
            E::PackageConfig { source } => Self::PackageConfig { source },
        }
    }
}

impl From<skiff_compiler_source::SourceCompileError> for PublicationError {
    fn from(error: skiff_compiler_source::SourceCompileError) -> Self {
        match error {
            skiff_compiler_source::SourceCompileError::Parse { path, source } => {
                Self::Parse { path, source }
            }
            skiff_compiler_source::SourceCompileError::ContractValidation { message } => {
                Self::ContractValidation { message }
            }
            skiff_compiler_source::SourceCompileError::RootPathReference { path, message } => {
                Self::RootPathReference { path, message }
            }
        }
    }
}

impl From<skiff_compiler_projection::error::ProjectionError> for PublicationError {
    fn from(error: skiff_compiler_projection::error::ProjectionError) -> Self {
        match error {
            skiff_compiler_projection::error::ProjectionError::NoExportedInterfaces => {
                Self::NoExportedInterfaces
            }
            skiff_compiler_projection::error::ProjectionError::ContractValidation { message } => {
                Self::ContractValidation { message }
            }
            skiff_compiler_projection::error::ProjectionError::ImplementationConformance {
                message,
            } => Self::ImplementationConformance { message },
        }
    }
}

impl From<skiff_compiler_emission::error::EmissionError> for PublicationError {
    fn from(error: skiff_compiler_emission::error::EmissionError) -> Self {
        match error {
            skiff_compiler_emission::error::EmissionError::ContractValidation { message } => {
                Self::ContractValidation { message }
            }
            skiff_compiler_emission::error::EmissionError::ArtifactIdentity { source } => {
                Self::ContractValidation {
                    message: source.to_string(),
                }
            }
        }
    }
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackageCompileError {
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
    #[error("contract validation failed:\n{message}")]
    ContractValidation { message: String },
    #[error("invalid root reference in {path}:\n{message}")]
    RootPathReference { path: String, message: String },
}

impl From<skiff_compiler_source::SourceCompileError> for PackageCompileError {
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

pub(crate) fn package_projection_error(
    error: skiff_compiler_projection::error::ProjectionError,
) -> PackageCompileError {
    match error {
        skiff_compiler_projection::error::ProjectionError::InvalidPackageArtifact { message } => {
            PackageCompileError::ContractValidation { message }
        }
    }
}

impl From<skiff_compiler_emission::error::EmissionError> for PackageCompileError {
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

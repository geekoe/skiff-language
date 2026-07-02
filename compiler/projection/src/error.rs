use thiserror::Error;

pub type Result<T> = std::result::Result<T, CompileError>;

#[derive(Debug, Clone, Error)]
pub enum CompileError {
    #[error("{0}")]
    Semantic(String),
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("service publication exports no interfaces")]
    NoExportedInterfaces,
    #[error("contract validation failed:\n{message}")]
    ContractValidation { message: String },
    #[error("service implementation conformance failed:\n{message}")]
    ImplementationConformance { message: String },
}

impl From<CompileError> for ProjectionError {
    fn from(error: CompileError) -> Self {
        Self::ContractValidation {
            message: error.to_string(),
        }
    }
}

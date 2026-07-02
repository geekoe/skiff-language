use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublicationError {
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

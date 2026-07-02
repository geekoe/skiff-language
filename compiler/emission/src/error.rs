use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmissionError {
    #[error("contract validation failed:\n{message}")]
    ContractValidation { message: String },
    #[error("{source}")]
    ArtifactIdentity {
        #[from]
        source: skiff_artifact_identity::ArtifactIdentityError,
    },
}

pub type Result<T> = std::result::Result<T, EmissionError>;

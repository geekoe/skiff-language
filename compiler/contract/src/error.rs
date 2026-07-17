use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractDefinitionError {
    #[error("ServiceContractDefinition operations must contain at least one operation")]
    EmptyOperations,
    #[error("ServiceContractDefinition {kind} key must be a non-empty string")]
    EmptyStableKey { kind: &'static str },
    #[error("contract diagnostic text references unknown operation stable key {key}")]
    UnknownDiagnosticOperation { key: String },
    #[error("contract diagnostic text references unknown type stable key {key}")]
    UnknownDiagnosticType { key: String },
    #[error(transparent)]
    Identity(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

pub type Result<T> = std::result::Result<T, ContractDefinitionError>;

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
    #[error("service API projection has no public callable for boundary projection {callable_id}")]
    MissingPublicCallable { callable_id: String },
    #[error("service API projection has no boundary projection for public callable {callable_id}")]
    MissingBoundaryProjection { callable_id: String },
    #[error(
        "service API projection maps callable {callable_id} to multiple public paths: {first}, {second}"
    )]
    DuplicatePublicCallable {
        callable_id: String,
        first: String,
        second: String,
    },
    #[error(transparent)]
    Identity(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

pub type Result<T> = std::result::Result<T, ContractDefinitionError>;

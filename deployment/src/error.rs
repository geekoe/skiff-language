use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error(transparent)]
    Artifact(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

pub type Result<T> = std::result::Result<T, DeploymentError>;

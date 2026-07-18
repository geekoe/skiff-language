use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("package artifact projection failed:\n{message}")]
    InvalidPackageArtifact { message: String },
}

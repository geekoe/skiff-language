use skiff_artifact_model::RuntimeConfigSnapshotRef;

use crate::RuntimeConfigSnapshot;

/// Backend-neutral lookup contract. A future encrypted store can implement
/// this trait without changing activation or runtime consumers.
pub trait RuntimeConfigSnapshotResolver: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn resolve(
        &self,
        reference: &RuntimeConfigSnapshotRef,
    ) -> Result<RuntimeConfigSnapshot, Self::Error>;
}

//! Release pointer resolution: `(profile, serviceId, version) -> buildId`.
//!
//! In the lazy-load deployment model the release pointer table is the only
//! mutable deployment state; this module owns the router read path from the
//! human coordinate to the exact immutable `ServiceDeploymentRef` (whose
//! `deployment_artifact_identity` is the buildId consumed by the runtime).

use skiff_artifact_model::ServiceDeploymentRef;
use skiff_deployment::storage::CanonicalArtifactStore;

/// Resolves one release pointer to the exact immutable deployment reference.
///
/// `Ok(None)` = the pointer is not set; `Err` = the pointer or its target
/// record cannot be read or validated. Callers treat both as fail-closed.
pub trait ReleaseResolver: Send + Sync {
    fn resolve(
        &self,
        profile: &str,
        service_id: &str,
        version: &str,
    ) -> Result<Option<ServiceDeploymentRef>, String>;
}

/// Production resolver over the canonical artifact store.
///
/// `read_release_pointer` itself fails closed when the target deployment
/// record is absent or invalid, so a resolved `Some` always names a readable
/// record.
#[derive(Debug, Clone)]
pub struct StoreReleaseResolver {
    artifact_store: CanonicalArtifactStore,
}

impl StoreReleaseResolver {
    pub fn new(artifact_store: CanonicalArtifactStore) -> Self {
        Self { artifact_store }
    }
}

impl ReleaseResolver for StoreReleaseResolver {
    fn resolve(
        &self,
        profile: &str,
        service_id: &str,
        version: &str,
    ) -> Result<Option<ServiceDeploymentRef>, String> {
        let pointer = self
            .artifact_store
            .read_release_pointer(profile, service_id, version)
            .map_err(|error| {
                format!("read release pointer {profile} {service_id} {version}: {error}")
            })?;
        Ok(pointer.map(|pointer| pointer.deployment))
    }
}

#[cfg(test)]
mod tests;

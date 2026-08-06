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

    /// Pointer-table scan: every deployment reference currently published
    /// for one profile (the surface-rebuild / health projection source; M4).
    fn all_deployments(&self, profile: &str) -> Result<Vec<ServiceDeploymentRef>, String>;
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

    fn all_deployments(&self, profile: &str) -> Result<Vec<ServiceDeploymentRef>, String> {
        let root = self.artifact_store.root().to_path_buf();
        let base = root
            .join("pointers")
            .join("releases")
            .join(profile_segment(profile)?);
        let mut deployments = Vec::new();
        if !base.exists() {
            return Ok(deployments);
        }
        let services = std::fs::read_dir(&base).map_err(|error| {
            format!("scan release pointers for {profile}: {error}")
        })?;
        for service in services {
            let service = service.map_err(|error| {
                format!("scan release pointers for {profile}: {error}")
            })?;
            let service_dir = service.path();
            if !service_dir.is_dir() {
                continue;
            }
            let versions = std::fs::read_dir(&service_dir).map_err(|error| {
                format!("scan release pointers for {profile}: {error}")
            })?;
            for version in versions {
                let version = version.map_err(|error| {
                    format!("scan release pointers for {profile}: {error}")
                })?;
                let version_file = version.path();
                if version_file.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let service_id = decode_segment(
                    service_dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| "release pointer service segment is not UTF-8".to_string())?,
                );
                let version = decode_segment(
                    version_file
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| "release pointer version segment is not UTF-8".to_string())?,
                );
                let pointer = self
                    .artifact_store
                    .read_release_pointer(profile, &service_id, &version)
                    .map_err(|error| {
                        format!(
                            "read release pointer {profile} {service_id} {version}: {error}"
                        )
                    })?;
                if let Some(pointer) = pointer {
                    deployments.push(pointer.deployment);
                }
            }
        }
        deployments.sort_by(|left, right| {
            left.service_id
                .cmp(&right.service_id)
                .then(left.contract_version.cmp(&right.contract_version))
        });
        Ok(deployments)
    }
}

/// Validates one profile as a single path segment (mirror of the artifact
/// identity release pointer path rule).
fn profile_segment(profile: &str) -> Result<String, String> {
    if profile.is_empty()
        || profile.len() > 200
        || profile != profile.trim()
        || profile == "."
        || profile == ".."
        || profile.contains('/')
        || profile.contains('\\')
        || profile.bytes().any(|byte| {
            !matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'
            )
        })
    {
        return Err(format!("profile {profile:?} is not a valid release pointer segment"));
    }
    Ok(profile.to_string())
}

/// Reverses the artifact identity coordinate segment encoding
/// (`.` -> `~d`, `/` -> `~s`).
fn decode_segment(value: &str) -> String {
    value.replace("~s", "/").replace("~d", ".")
}

#[cfg(test)]
mod tests;

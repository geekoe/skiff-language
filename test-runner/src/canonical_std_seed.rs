use std::path::Path;

use serde_json::{json, Value};
use skiff_artifact_identity::{package_artifact_ref, PackageArtifactPointerPath};
use skiff_compiler::{
    authoring::{
        author_official_std_package, publish_package_artifact_records,
        PublishedPackageArtifactReceipt,
    },
    CompilerPlatformSources,
};
use skiff_deployment::storage::{
    CanonicalArtifactStore, EcosystemStorageError, PackageArtifactPointer,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalStdSeedReceipt {
    pub package: PublishedPackageArtifactReceipt,
    pub pointer: PackageArtifactPointer,
    pub pointer_path: String,
}

impl CanonicalStdSeedReceipt {
    pub fn to_json(&self) -> Value {
        json!({
            "package": self.package,
            "pointer": self.pointer,
            "pointerPath": self.pointer_path,
        })
    }
}

#[derive(Debug, Error)]
pub enum CanonicalStdSeedError {
    #[error("official std authoring failed: {0}")]
    Authoring(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Artifact(#[from] skiff_artifact_identity::ArtifactIdentityError),
    #[error(transparent)]
    Storage(#[from] EcosystemStorageError),
    #[error("canonical std pointer already selects {actual:?}; exact candidate is {candidate:?}")]
    ConflictingPointer {
        actual: Box<PackageArtifactPointer>,
        candidate: Box<PackageArtifactPointer>,
    },
}

/// Idempotently installs the compiler-owned std candidate into one canonical store.
///
/// Existing pointer state is validated before any immutable record write. Records
/// are published before the absent-pointer CAS, so a crash can leave only
/// recoverable orphan records and never a pointer to missing records.
pub fn seed_canonical_std(
    platform_sources: &CompilerPlatformSources,
    artifact_root: &Path,
) -> Result<CanonicalStdSeedReceipt, CanonicalStdSeedError> {
    let published =
        author_official_std_package(platform_sources).map_err(CanonicalStdSeedError::Authoring)?;
    let artifact = package_artifact_ref(&published.artifact)?;
    let candidate = PackageArtifactPointer::new(artifact)?;
    let store = CanonicalArtifactStore::create(artifact_root)?;
    let current = store.read_package_artifact_pointer(
        &candidate.artifact.package_id,
        &candidate.artifact.package_version,
    )?;
    if let Some(actual) = current.as_ref() {
        if actual != &candidate {
            return Err(CanonicalStdSeedError::ConflictingPointer {
                actual: Box::new(actual.clone()),
                candidate: Box::new(candidate),
            });
        }
    }

    let package = publish_package_artifact_records(&store, &published)
        .map_err(CanonicalStdSeedError::Authoring)?;
    if current.is_none() {
        match store.compare_and_swap_package_artifact_pointer(None, &candidate) {
            Ok(()) => {}
            Err(EcosystemStorageError::CasMismatch { .. }) => {
                let actual = store
                    .read_package_artifact_pointer(
                        &candidate.artifact.package_id,
                        &candidate.artifact.package_version,
                    )?
                    .ok_or_else(|| EcosystemStorageError::CasMismatch {
                        path: store.root().to_path_buf(),
                        message: "canonical std pointer disappeared after concurrent CAS"
                            .to_string(),
                    })?;
                if actual != candidate {
                    return Err(CanonicalStdSeedError::ConflictingPointer {
                        actual: Box::new(actual),
                        candidate: Box::new(candidate),
                    });
                }
            }
            Err(error) => return Err(error.into()),
        }
    }

    let pointer_path = PackageArtifactPointerPath::new(
        &candidate.artifact.package_id,
        &candidate.artifact.package_version,
    )?
    .to_string();
    Ok(CanonicalStdSeedReceipt {
        package,
        pointer: candidate,
        pointer_path,
    })
}

#[cfg(test)]
mod tests;

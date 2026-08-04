//! Strict bootstrap loader (C-bootstrap §2.3, C-model-artifact §3).
//!
//! Loads the validated `RuntimeAssembly` + `RuntimeConfigSnapshot` + A3 actor
//! routing projection and builds the complete immutable `RoutingEpoch`.
//! Every read goes through the owner stores' strict chains; any failure
//! produces a `BootstrapLoadFailure` and never a partial epoch. The loader is
//! sync and `Send + Sync`; callers must invoke it through `BlockingLoader`.

use std::path::Path;
use std::sync::Arc;

use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore;

use crate::artifact::{
    ActorRoutingCatalog, ActorRoutingProjectionError, ActorRoutingProjectionRef,
    ActorRoutingProjectionStore,
};

use super::epoch::RoutingEpoch;

/// Fail-closed load failures; no partial epoch is ever produced.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapLoadFailure {
    #[error("bootstrap loader open failed: {0}")]
    Open(String),
    #[error("runtime assembly strict read failed: {0}")]
    Assembly(String),
    #[error("runtime config snapshot strict read failed: {0}")]
    Snapshot(String),
    #[error("actor routing projection strict read failed: {0}")]
    ActorProjection(String),
    #[error("routing epoch profile mismatch: expected {expected}, snapshot profile {actual}")]
    ProfileMismatch { expected: String, actual: String },
    #[error("routing epoch validation failed: {0}")]
    InvalidEpoch(String),
}

impl BootstrapLoadFailure {
    pub(crate) fn invalid_epoch(message: String) -> Self {
        Self::InvalidEpoch(message)
    }

    pub fn from_actor_projection(error: ActorRoutingProjectionError) -> Self {
        Self::ActorProjection(error.to_string())
    }
}

/// Strict loader composition over the canonical stores.
#[derive(Debug, Clone)]
pub struct BootstrapStrictLoader {
    assembly_store: CanonicalArtifactStore,
    snapshot_store: RuntimeConfigSnapshotStore,
    actor_projection_store: ActorRoutingProjectionStore,
}

impl BootstrapStrictLoader {
    /// Opens the canonical artifact root and the snapshot store.
    ///
    /// The snapshot store root may be the same directory as the artifact root
    /// (snapshot records live under `<root>/snapshots/`); callers may pass
    /// distinct roots for tests.
    pub fn open(
        artifact_root: impl AsRef<Path>,
        snapshot_root: impl AsRef<Path>,
    ) -> Result<Self, BootstrapLoadFailure> {
        let assembly_store =
            CanonicalArtifactStore::open(artifact_root.as_ref()).map_err(|error| {
                BootstrapLoadFailure::Open(format!(
                    "open canonical artifact store at {}: {error}",
                    artifact_root.as_ref().display()
                ))
            })?;
        let snapshot_store =
            RuntimeConfigSnapshotStore::open(snapshot_root.as_ref()).map_err(|error| {
                BootstrapLoadFailure::Open(format!(
                    "open runtime config snapshot store at {}: {error}",
                    snapshot_root.as_ref().display()
                ))
            })?;
        let actor_projection_store = ActorRoutingProjectionStore::open(artifact_root.as_ref())
            .map_err(|error| {
                BootstrapLoadFailure::Open(format!(
                    "open actor routing projection store at {}: {error}",
                    artifact_root.as_ref().display()
                ))
            })?;
        Ok(Self {
            assembly_store,
            snapshot_store,
            actor_projection_store,
        })
    }

    pub fn artifact_root(&self) -> &Path {
        self.assembly_store.root()
    }

    pub fn snapshot_root(&self) -> &Path {
        self.snapshot_store.root()
    }

    /// Strict load order (C-bootstrap §2.3): assembly → snapshot → snapshot
    /// profile check → actor routing projection/catalog → epoch.
    ///
    /// `actor_projection` is the A3 typed record ref (canonical derivation is
    /// the A1/integration alignment seam; the reader/loader chain never
    /// guesses paths).
    pub fn load_epoch(
        &self,
        profile: &str,
        generation: u64,
        assembly_ref: &skiff_artifact_model::RuntimeAssemblyRef,
        snapshot_ref: &skiff_artifact_model::RuntimeConfigSnapshotRef,
        actor_projection: &ActorRoutingProjectionRef,
    ) -> Result<Arc<RoutingEpoch>, BootstrapLoadFailure> {
        let assembly = self
            .assembly_store
            .read_runtime_assembly(assembly_ref)
            .map_err(|error| BootstrapLoadFailure::Assembly(error.to_string()))?;
        let snapshot = self
            .snapshot_store
            .read(snapshot_ref)
            .map_err(|error| BootstrapLoadFailure::Snapshot(error.to_string()))?;
        if snapshot.profile() != profile {
            return Err(BootstrapLoadFailure::ProfileMismatch {
                expected: profile.to_string(),
                actual: snapshot.profile().to_string(),
            });
        }
        let projection = self
            .actor_projection_store
            .load(actor_projection)
            .map_err(BootstrapLoadFailure::from_actor_projection)?;
        let catalog = Arc::new(ActorRoutingCatalog::from_projection(projection));
        let epoch = RoutingEpoch::new(profile, generation, assembly, Arc::new(snapshot), catalog)?;
        Ok(Arc::new(epoch))
    }
}

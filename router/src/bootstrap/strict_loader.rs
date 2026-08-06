//! Strict actor routing projection loader (M4: on-demand reads).
//!
//! The actor catalog is no longer built into a routing epoch at bootstrap.
//! [`BootstrapStrictLoader`] opens the actor routing projection store at the
//! artifact root and loads the catalog on demand; the actor catalog view
//! caches the loaded catalog. Reads go through the owner store's strict
//! chain; any failure is a `BootstrapLoadFailure`.

use std::path::Path;
use std::sync::Arc;

use crate::artifact::{
    ActorRoutingCatalog, ActorRoutingProjectionError, ActorRoutingProjectionRef,
    ActorRoutingProjectionStore,
};

/// Fail-closed load failures; no partial catalog is ever produced.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapLoadFailure {
    #[error("bootstrap loader open failed: {0}")]
    Open(String),
    #[error("actor routing projection strict read failed: {0}")]
    ActorProjection(String),
}

impl BootstrapLoadFailure {
    pub fn from_actor_projection(error: ActorRoutingProjectionError) -> Self {
        Self::ActorProjection(error.to_string())
    }
}

/// Strict loader composition over the canonical stores.
#[derive(Debug, Clone)]
pub struct BootstrapStrictLoader {
    actor_projection_store: ActorRoutingProjectionStore,
}

impl BootstrapStrictLoader {
    /// Opens the actor routing projection store at the canonical artifact
    /// root.
    pub fn open(artifact_root: impl AsRef<Path>) -> Result<Self, BootstrapLoadFailure> {
        let actor_projection_store = ActorRoutingProjectionStore::open(artifact_root.as_ref())
            .map_err(|error| {
                BootstrapLoadFailure::Open(format!(
                    "open actor routing projection store at {}: {error}",
                    artifact_root.as_ref().display()
                ))
            })?;
        Ok(Self {
            actor_projection_store,
        })
    }

    pub fn artifact_root(&self) -> &Path {
        self.actor_projection_store.root()
    }

    /// Loads the validated actor routing catalog for one projection record
    /// (the actor catalog view reads through this on demand).
    pub fn load_actor_catalog(
        &self,
        actor_projection: &ActorRoutingProjectionRef,
    ) -> Result<Arc<ActorRoutingCatalog>, BootstrapLoadFailure> {
        self.actor_projection_store
            .load_catalog(actor_projection)
            .map_err(BootstrapLoadFailure::from_actor_projection)
    }
}

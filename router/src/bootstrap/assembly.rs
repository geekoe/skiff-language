//! E-bootstrap production assembly (M4: no committed/pending state, no Mongo
//! repository).
//!
//! This is the only production wiring owner for the initial bootstrap chain:
//! open the CanonicalArtifactStore + validate the profile. Fail-closed
//! outcomes are exactly two: the artifact store cannot be opened, or the
//! configured profile is invalid. No deployment state is loaded at startup;
//! deployments resolve on demand through the release pointer table
//! (`ReleaseResolver`).

use std::{fmt, sync::Arc};

use skiff_artifact_identity::ArtifactRelativePath;
use skiff_artifact_model::validate_activation_profile;
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::artifact::ActorRoutingProjectionRef;
use crate::config::RouterConfig;

use super::loader::{BlockingLoader, BlockingLoaderOptions};

/// Relative record path of the actor routing projection inside the canonical
/// artifact root.
///
/// The projection data contract is frozen by the A0/A3 projection; this is
/// the consumer-side record path the actor catalog view reads on demand.
pub const ACTOR_ROUTING_PROJECTION_RECORD_PATH: &str = "records/actor-routing/current.json";

/// Fail-closed bootstrap assembly errors; no listener is started.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapAssemblyError {
    #[error("canonical artifact store open failed: {0}")]
    Store(String),
    #[error("actor routing projection record path is invalid: {0}")]
    ActorProjectionPath(String),
    #[error("router profile is invalid: {0}")]
    Profile(String),
}

/// Assembled E-bootstrap state: the opened canonical artifact store, the
/// validated profile, and the bounded blocking loader used for on-demand
/// artifact reads (actor catalog / release resolution).
pub struct RouterBootstrapAssembly {
    profile: String,
    store: CanonicalArtifactStore,
    loader: Arc<BlockingLoader>,
    actor_projection: ActorRoutingProjectionRef,
}

impl fmt::Debug for RouterBootstrapAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouterBootstrapAssembly")
            .field("profile", &self.profile)
            .field("store_root", &self.store.root())
            .field("loader", &self.loader)
            .field("actor_projection", &self.actor_projection)
            .finish()
    }
}

impl RouterBootstrapAssembly {
    /// Runs the full initial bootstrap from the frozen Router config.
    ///
    /// Opens the canonical artifact store and validates the profile. On any
    /// fail-closed outcome no assembly is produced.
    pub async fn assemble(config: &RouterConfig) -> Result<Self, BootstrapAssemblyError> {
        let profile = config.profile.clone();
        validate_activation_profile(&profile)
            .map_err(|error| BootstrapAssemblyError::Profile(error.to_string()))?;
        let store = CanonicalArtifactStore::open(&config.artifacts_path)
            .map_err(|error| BootstrapAssemblyError::Store(error.to_string()))?;
        let loader = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
        let actor_projection = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .map_err(|error| BootstrapAssemblyError::ActorProjectionPath(error.to_string()))?,
        );
        Ok(Self {
            profile,
            store,
            loader,
            actor_projection,
        })
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The opened canonical artifact store. All on-demand deployment reads
    /// (release pointers, deployment records, actor routing projection) go
    /// through this store.
    pub fn store(&self) -> &CanonicalArtifactStore {
        &self.store
    }

    pub fn loader(&self) -> Arc<BlockingLoader> {
        Arc::clone(&self.loader)
    }

    pub fn actor_projection(&self) -> ActorRoutingProjectionRef {
        self.actor_projection.clone()
    }

    /// Drains the blocking loader. Called after the listeners and sessions
    /// have shut down; idempotent.
    pub async fn shutdown(&self) {
        self.loader.shutdown().await;
    }
}

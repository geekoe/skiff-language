//! E-bootstrap production assembly (plan §7 E-bootstrap, C-bootstrap §2.5).
//!
//! This is the only production wiring owner for the initial bootstrap chain:
//! repository (W-activation-state `ActivationStateRepository` read side) →
//! `CommittedActivationBootstrapReader` → strict loader →
//! `ActiveRoutingEpochStore`. The committed epoch must be published before the
//! listeners are started; pending / missing / malformed / identity mismatch /
//! loader failures all fail closed with no listener and no partial epoch.
//! Full cold recovery belongs to E-activation.

use std::{fmt, sync::Arc};

use skiff_artifact_identity::ArtifactRelativePath;

use crate::activation::{
    ActivationStateRepository, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, SystemClock,
};
use crate::artifact::ActorRoutingProjectionRef;
use crate::config::RouterConfig;

use super::epoch::{ActiveRoutingEpochStore, RoutingEpoch};
use super::loader::{BlockingLoader, BlockingLoaderOptions};
use super::reader::{CanonicalCommittedRefValidator, CommittedActivationBootstrapReader};
use super::runner::{BootstrapError, BootstrapHealthSnapshot, BootstrapRunner};
use super::strict_loader::{BootstrapLoadFailure, BootstrapStrictLoader};

/// Relative record path of the actor routing projection inside the canonical
/// artifact root.
///
/// A0/A3 freeze the projection data contract but not the record identity/path
/// derivation (A1 producer output surface). Until that producer is wired,
/// E-bootstrap is the integration seam and makes the path explicit; the
/// strict reader/loader validation chain is unchanged and a missing or
/// non-canonical record still fails closed.
pub const ACTOR_ROUTING_PROJECTION_RECORD_PATH: &str = "records/actor-routing/current.json";

/// Fail-closed bootstrap assembly errors; no listener is started.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapAssemblyError {
    #[error("router config environment is required for E-bootstrap")]
    EnvironmentMissing,
    #[error("activation state repository connect failed: {0}")]
    Repository(String),
    #[error("committed ref validator open failed: {0}")]
    Validator(String),
    #[error("strict loader open failed: {0}")]
    StrictLoader(#[from] BootstrapLoadFailure),
    #[error("actor routing projection record path is invalid: {0}")]
    ActorProjectionPath(String),
    #[error("bootstrap failed closed: {0}")]
    Bootstrap(#[from] BootstrapError),
}

/// Assembled E-bootstrap state: the published epoch, its single-authority
/// store, and the owned blocking loader / repository shut down with the
/// process.
pub struct RouterBootstrapAssembly {
    environment: String,
    epoch: Arc<RoutingEpoch>,
    epoch_store: Arc<ActiveRoutingEpochStore>,
    loader: Arc<BlockingLoader>,
    strict_loader: Arc<BootstrapStrictLoader>,
    actor_projection: ActorRoutingProjectionRef,
    runner: BootstrapRunner,
    repository: Arc<dyn ActivationStateRepository>,
}

impl fmt::Debug for RouterBootstrapAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouterBootstrapAssembly")
            .field("environment", &self.environment)
            .field("epoch", &self.epoch)
            .field("epoch_store", &self.epoch_store)
            .field("loader", &self.loader)
            .field("strict_loader", &self.strict_loader)
            .field("actor_projection", &self.actor_projection)
            .field("runner", &self.runner)
            .field("repository", &"<activation state repository>")
            .finish()
    }
}

impl RouterBootstrapAssembly {
    /// Runs the full initial bootstrap chain from the frozen Router config.
    ///
    /// On success the committed epoch is published and the assembly is ready
    /// for listener startup; on any fail-closed outcome the repository is
    /// closed and no epoch is published.
    pub async fn assemble(config: &RouterConfig) -> Result<Self, BootstrapAssemblyError> {
        let environment = config
            .environment
            .clone()
            .ok_or(BootstrapAssemblyError::EnvironmentMissing)?;
        let repository = Arc::new(
            MongoActivationStateRepository::connect(
                &config.service_db.mongo_url,
                MongoActivationStateRepositoryOptions::default(),
                Arc::new(SystemClock),
            )
            .await
            .map_err(|error| BootstrapAssemblyError::Repository(error.to_string()))?,
        ) as Arc<dyn ActivationStateRepository>;
        match Self::assemble_with(config, &environment, Arc::clone(&repository)).await {
            Ok(assembly) => Ok(assembly),
            Err(error) => {
                let _ = repository.close().await;
                Err(error)
            }
        }
    }

    /// Assembly with an injected repository (tests use the memory fake; the
    /// production entry point connects the Mongo adapter).
    pub async fn assemble_with(
        config: &RouterConfig,
        environment: &str,
        repository: Arc<dyn ActivationStateRepository>,
    ) -> Result<Self, BootstrapAssemblyError> {
        let validator = Arc::new(
            CanonicalCommittedRefValidator::open(&config.artifacts_path)
                .map_err(BootstrapAssemblyError::Validator)?,
        );
        let loader = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
        let reader = CommittedActivationBootstrapReader::new(
            Arc::clone(&repository),
            validator,
            Arc::clone(&loader),
        );
        // The canonical snapshot producer (`config-snapshot-tooling`) publishes
        // under `<artifact-root>/runtime-config`; open the snapshot store at
        // that subroot while the assembly/actor records stay at the artifact
        // root.
        let snapshot_root = config.artifacts_path.join("runtime-config");
        let strict_loader = Arc::new(BootstrapStrictLoader::open(
            &config.artifacts_path,
            &snapshot_root,
        )?);
        let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
        let runner = BootstrapRunner::new(
            reader,
            Arc::clone(&strict_loader),
            Arc::clone(&loader),
            Arc::clone(&epoch_store),
        );
        let actor_projection = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .map_err(|error| BootstrapAssemblyError::ActorProjectionPath(error.to_string()))?,
        );
        let epoch = runner.run_initial(environment, &actor_projection).await?;
        Ok(Self {
            environment: environment.to_string(),
            epoch,
            epoch_store,
            loader,
            strict_loader,
            actor_projection,
            runner,
            repository,
        })
    }

    pub fn epoch_store(&self) -> Arc<ActiveRoutingEpochStore> {
        Arc::clone(&self.epoch_store)
    }

    pub fn environment(&self) -> &str {
        &self.environment
    }

    pub fn epoch(&self) -> &Arc<RoutingEpoch> {
        &self.epoch
    }

    pub fn loader(&self) -> Arc<BlockingLoader> {
        Arc::clone(&self.loader)
    }

    pub fn strict_loader(&self) -> Arc<BootstrapStrictLoader> {
        Arc::clone(&self.strict_loader)
    }

    pub fn actor_projection(&self) -> ActorRoutingProjectionRef {
        self.actor_projection.clone()
    }

    pub fn repository(&self) -> Arc<dyn ActivationStateRepository> {
        Arc::clone(&self.repository)
    }

    pub fn health(&self) -> BootstrapHealthSnapshot {
        self.runner.health()
    }

    /// Drains the blocking loader and closes the repository. Called after the
    /// listeners and sessions have shut down; idempotent.
    pub async fn shutdown(&self) {
        self.loader.shutdown().await;
        let _ = self.repository.close().await;
    }
}

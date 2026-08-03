//! Initial bootstrap orchestration (C-bootstrap §2.5, plan §7 E-bootstrap).
//!
//! `BootstrapRunner` owns the read → project → strict load → publish chain.
//! A stable committed outcome (with or without a durable pending
//! activation) publishes the committed epoch; missing/malformed/identity
//! mismatch/loader failures still fail close with an empty store. Durable
//! pending recovery is installed by the activation coordinator (§4.2); the
//! runner only surfaces the pending record to the assembly.

use std::sync::Arc;

use skiff_deployment::activation_state::PendingActivation;

use crate::artifact::ActorRoutingProjectionRef;

use super::epoch::{ActiveRoutingEpochStore, RoutingEpoch, RoutingEpochHealth};
use super::loader::{BlockingLoader, BlockingLoaderError, BlockingLoaderHealth};
use super::reader::{
    BootstrapReadOutcome, CommittedActivationBootstrapReader, CommittedBootstrapRefs,
    ReaderFailClosedCounters,
};
use super::strict_loader::{BootstrapLoadFailure, BootstrapStrictLoader};

/// Fail-closed bootstrap errors; no partial epoch is ever published.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("bootstrap read failed closed: {0}")]
    Read(BootstrapReadOutcome),
    #[error("bootstrap strict load failed: {0}")]
    Load(#[from] BootstrapLoadFailure),
    #[error("blocking loader failed: {0}")]
    Loader(String),
}

/// Successful initial bootstrap result: the published committed epoch plus
/// the durable pending activation (if any) that E-activation must install as
/// a recovery transaction (plan §4.2).
#[derive(Debug)]
pub struct BootstrapRunOutcome {
    pub epoch: Arc<RoutingEpoch>,
    pub pending: Option<PendingActivation>,
}

/// Health snapshot combining the epoch store, reader counters and loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapHealthSnapshot {
    pub active_epoch: Option<RoutingEpochHealth>,
    pub reader_fail_closed: ReaderFailClosedCounters,
    pub loader: BlockingLoaderHealth,
    pub epoch_store_publish_count: u64,
}

/// Composed initial bootstrap runner.
#[derive(Debug)]
pub struct BootstrapRunner {
    reader: CommittedActivationBootstrapReader,
    strict_loader: Arc<BootstrapStrictLoader>,
    loader: Arc<BlockingLoader>,
    epoch_store: Arc<ActiveRoutingEpochStore>,
}

impl BootstrapRunner {
    pub fn new(
        reader: CommittedActivationBootstrapReader,
        strict_loader: Arc<BootstrapStrictLoader>,
        loader: Arc<BlockingLoader>,
        epoch_store: Arc<ActiveRoutingEpochStore>,
    ) -> Self {
        Self {
            reader,
            strict_loader,
            loader,
            epoch_store,
        }
    }

    /// Runs the initial bootstrap for one environment.
    ///
    /// `actor_projection` is the A3 typed record ref (canonical derivation is
    /// the A1/integration alignment seam). On success the epoch is atomically
    /// published and returned; on any fail-closed outcome the store is left
    /// untouched.
    pub async fn run_initial(
        &self,
        environment: &str,
        actor_projection: &ActorRoutingProjectionRef,
    ) -> Result<BootstrapRunOutcome, BootstrapError> {
        let outcome = self.reader.read_committed(environment).await;
        let pending = match &outcome {
            BootstrapReadOutcome::CommittedWithPending { pending, .. } => Some(pending.clone()),
            _ => None,
        };
        let refs = match outcome {
            BootstrapReadOutcome::StableCommitted {
                generation,
                assembly,
                config_snapshot,
            } => CommittedBootstrapRefs {
                generation,
                assembly,
                config_snapshot,
            },
            BootstrapReadOutcome::CommittedWithPending { refs, .. } => refs,
            other => return Err(BootstrapError::Read(other)),
        };
        let strict_loader = Arc::clone(&self.strict_loader);
        let actor_projection = actor_projection.clone();
        let environment = environment.to_string();
        let epoch = self
            .loader
            .run(move || {
                strict_loader.load_epoch(
                    &environment,
                    refs.generation,
                    &refs.assembly,
                    &refs.config_snapshot,
                    &actor_projection,
                )
            })
            .await
            .map_err(|error| match error {
                BlockingLoaderError::Operation(load_failure) => BootstrapError::Load(load_failure),
                other => BootstrapError::Loader(other.to_string()),
            })?;
        self.epoch_store.publish(Arc::clone(&epoch));
        Ok(BootstrapRunOutcome { epoch, pending })
    }

    pub fn epoch_store(&self) -> &Arc<ActiveRoutingEpochStore> {
        &self.epoch_store
    }

    pub fn loader(&self) -> &Arc<BlockingLoader> {
        &self.loader
    }

    pub fn reader(&self) -> &CommittedActivationBootstrapReader {
        &self.reader
    }

    pub fn health(&self) -> BootstrapHealthSnapshot {
        let store_health = self.epoch_store.health();
        BootstrapHealthSnapshot {
            active_epoch: store_health.current,
            reader_fail_closed: self.reader.fail_closed(),
            loader: self.loader.health(),
            epoch_store_publish_count: store_health.publish_count,
        }
    }
}

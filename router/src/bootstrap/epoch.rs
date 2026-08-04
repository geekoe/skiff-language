//! Immutable routing epoch and its single-authority store (plan §3.3,
//! C-bootstrap §2.4).
//!
//! `RoutingEpoch` is immutable after construction: it captures the
//! strict-loaded assembly (ingress/deployment projection source), the
//! strict-loaded config snapshot and the once-built actor routing catalog.
//! `ActiveRoutingEpochStore` holds exactly one current epoch and publishes by
//! whole-pointer replacement; captured `Arc`s keep old epochs alive without a
//! global pin map.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{
    validate_activation_generation, validate_activation_profile, GatewayIngressBinding,
    RuntimeAssembly, ServiceDeploymentRef,
};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

use crate::artifact::ActorRoutingCatalog;
use crate::session::identity::RegisteredAssemblyTuple;

use super::strict_loader::BootstrapLoadFailure;

/// Complete immutable routing epoch (authority design §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingEpoch {
    profile: String,
    assembly_generation: u64,
    assembly: Arc<RuntimeAssembly>,
    snapshot: Arc<RuntimeConfigSnapshot>,
    actor_catalog: Arc<ActorRoutingCatalog>,
}

impl RoutingEpoch {
    /// Builds an epoch from strict-loaded inputs.
    ///
    /// Validation is fail-closed: the snapshot profile must exactly match
    /// the caller profile, and the generation/profile must satisfy the
    /// frozen lexical rules. All artifact inputs were already validated by the
    /// strict stores before this constructor is reached.
    pub fn new(
        profile: impl Into<String>,
        assembly_generation: u64,
        assembly: Arc<RuntimeAssembly>,
        snapshot: Arc<RuntimeConfigSnapshot>,
        actor_catalog: Arc<ActorRoutingCatalog>,
    ) -> Result<Self, BootstrapLoadFailure> {
        let profile = profile.into();
        validate_activation_profile(&profile).map_err(BootstrapLoadFailure::invalid_epoch)?;
        validate_activation_generation(assembly_generation, "assemblyGeneration")
            .map_err(BootstrapLoadFailure::invalid_epoch)?;
        if snapshot.profile() != profile {
            return Err(BootstrapLoadFailure::ProfileMismatch {
                expected: profile.clone(),
                actual: snapshot.profile().to_string(),
            });
        }
        Ok(Self {
            profile,
            assembly_generation,
            assembly,
            snapshot,
            actor_catalog,
        })
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn assembly_generation(&self) -> u64 {
        self.assembly_generation
    }

    pub fn assembly_identity(&self) -> &str {
        self.assembly.assembly_identity.as_str()
    }

    pub fn config_snapshot_id(&self) -> &str {
        self.snapshot.snapshot_ref().snapshot_id.as_str()
    }

    pub fn assembly(&self) -> &RuntimeAssembly {
        &self.assembly
    }

    pub fn snapshot(&self) -> &RuntimeConfigSnapshot {
        &self.snapshot
    }

    pub fn actor_catalog(&self) -> &ActorRoutingCatalog {
        &self.actor_catalog
    }

    /// Immutable ingress projection (from the strict-loaded assembly).
    pub fn ingress_projection(&self) -> &[GatewayIngressBinding] {
        &self.assembly.gateway_ingress
    }

    /// Immutable deployment projection (from the strict-loaded assembly).
    pub fn deployment_projection(&self) -> &[ServiceDeploymentRef] {
        &self.assembly.resolved_deployments
    }

    /// W-session seam mapping: the exact tuple a Runtime must register against
    /// this epoch (plan §3.2/§3.5).
    pub fn registered_tuple(&self) -> RegisteredAssemblyTuple {
        RegisteredAssemblyTuple {
            profile: self.profile.clone(),
            generation: self.assembly_generation,
            assembly: skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: self.assembly.assembly_identity.clone(),
            },
            config_snapshot: self.snapshot.snapshot_ref().clone(),
        }
    }
}

/// Read-only epoch projection for health (`activeRoutingEpoch.*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingEpochHealth {
    pub profile: String,
    pub assembly_generation: u64,
    pub assembly_identity: String,
    pub config_snapshot_id: String,
}

impl RoutingEpochHealth {
    pub fn from_epoch(epoch: &RoutingEpoch) -> Self {
        Self {
            profile: epoch.profile.clone(),
            assembly_generation: epoch.assembly_generation,
            assembly_identity: epoch.assembly_identity().to_string(),
            config_snapshot_id: epoch.config_snapshot_id().to_string(),
        }
    }
}

/// Single-authority store for the current immutable routing epoch.
///
/// Invariant (C-bootstrap §2.4/§4): at most one current epoch, always
/// complete/validated/immutable; capture and publish only ever move a whole
/// epoch pointer; pending/eligibility/cache never enter this store. The
/// replacement is a whole-pointer swap: old captured `Arc`s are never
/// cancelled or deleted by a later publish.
#[derive(Debug, Default)]
pub struct ActiveRoutingEpochStore {
    current: Mutex<Option<Arc<RoutingEpoch>>>,
    publish_count: AtomicU64,
}

impl ActiveRoutingEpochStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically replaces the current epoch with a complete validated epoch.
    ///
    /// Infallible and non-rollback: the epoch is immutable and the swap is a
    /// single whole-pointer replacement.
    pub fn publish(&self, epoch: Arc<RoutingEpoch>) {
        *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(epoch);
        self.publish_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Captures the current whole epoch, if published.
    pub fn capture(&self) -> Option<Arc<RoutingEpoch>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn publish_count(&self) -> u64 {
        self.publish_count.load(Ordering::Relaxed)
    }

    pub fn health(&self) -> EpochStoreHealth {
        let current = self
            .capture()
            .map(|epoch| RoutingEpochHealth::from_epoch(&epoch));
        EpochStoreHealth {
            publish_count: self.publish_count(),
            current,
        }
    }
}

/// Health projection of the epoch store (`epochStore.publishCount` +
/// `activeRoutingEpoch.*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochStoreHealth {
    pub publish_count: u64,
    pub current: Option<RoutingEpochHealth>,
}

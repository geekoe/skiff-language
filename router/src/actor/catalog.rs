//! `ActorMethodCatalogView`: stateless typed query over the A0/A3 actor index
//! inside one explicitly captured `Arc<RoutingEpoch>` (authority design
//! §3.2/§3.3, C-actor §3.1, C-model-actor §3).
//!
//! The view never reads PackageArtifact / File IR, never accepts source or
//! declaration coordinates as query input, never builds an independent index
//! and never refreshes: it queries the immutable catalog that belongs to the
//! captured epoch.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingRef, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

use crate::bootstrap::RoutingEpoch;

use super::health::CatalogHealth;

/// Typed method admission key (C-actor §3.1).
///
/// Deliberately contains only projection identities: no declarationOwner,
/// modulePath, actorName, methodName, sourceSpan or File IR coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogQuery {
    pub service_id: String,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
}

impl CatalogQuery {
    pub fn new(
        service_id: impl Into<String>,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        method_identity: ActorMethodIdentity,
    ) -> Self {
        Self {
            service_id: service_id.into(),
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        }
    }
}

/// Read-only projection view over one captured immutable epoch.
#[derive(Debug, Clone)]
pub struct ActorMethodCatalogView {
    epoch: Arc<RoutingEpoch>,
    captures: Arc<AtomicU64>,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
}

impl ActorMethodCatalogView {
    /// Captures one explicit epoch lease. The epoch is never replaced under
    /// the view; an old captured `Arc` keeps its whole index alive.
    pub fn new(epoch: Arc<RoutingEpoch>) -> Self {
        Self {
            epoch,
            captures: Arc::new(AtomicU64::new(1)),
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn from_epoch(epoch: Arc<RoutingEpoch>) -> Self {
        Self::new(epoch)
    }

    /// The captured epoch lease backing this view.
    pub fn epoch(&self) -> &Arc<RoutingEpoch> {
        &self.epoch
    }

    /// Projection schema version of the captured epoch (C-actor §7).
    pub fn schema_version(&self) -> &str {
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
    }

    /// Exact typed-key hit test.
    pub fn has_method(&self, query: &CatalogQuery) -> bool {
        self.method_for(query).is_some()
    }

    /// Exact typed-key lookup returning the immutable method entry with its
    /// deployment/package binding.
    pub fn method_for(&self, query: &CatalogQuery) -> Option<ActorRoutingMethod> {
        let found = self
            .epoch
            .actor_catalog()
            .entries()
            .iter()
            .find(|entry| matches_key(entry, query))
            .cloned();
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    pub fn health(&self) -> CatalogHealth {
        CatalogHealth {
            captures: self.captures.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

fn matches_key(entry: &ActorRoutingMethod, query: &CatalogQuery) -> bool {
    let actor = ActorRoutingRef {
        service_id: query.service_id.clone(),
        actor_abi_identity: query.actor_abi_identity.clone(),
    };
    entry.actor == actor
        && entry.actor_implementation_identity == query.actor_implementation_identity
        && entry.method_identity == query.method_identity
}

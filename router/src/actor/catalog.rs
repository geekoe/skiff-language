//! `ActorMethodCatalogView`: stateless typed query over the A0/A3 actor index
//! loaded on demand from the artifact store (M4: no routing epoch).
//!
//! The view reads the actor routing projection record from the artifact
//! store through the bounded blocking loader, builds the immutable catalog
//! once and caches it for the process lifetime. The view never reads
//! PackageArtifact / File IR, never accepts source or declaration
//! coordinates as query input, never builds an independent index and never
//! refreshes beyond the single on-demand load.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingRef, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

use crate::artifact::{
    ActorRoutingCatalog, ActorRoutingProjectionError, ActorRoutingProjectionRef,
    ActorRoutingProjectionStore,
};

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

/// Read-only projection view over the lazily loaded actor routing catalog.
///
/// The catalog is loaded once on first query (through the blocking loader)
/// and cached; a load failure fails the query closed (`None`) and the next
/// query retries the load.
#[derive(Debug)]
pub struct ActorMethodCatalogView {
    projection_store: ActorRoutingProjectionStore,
    actor_projection: ActorRoutingProjectionRef,
    catalog: Mutex<Option<Arc<ActorRoutingCatalog>>>,
    captures: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    loads: AtomicU64,
}

impl ActorMethodCatalogView {
    /// Builds a view over the artifact root and the actor routing projection
    /// record; the catalog is loaded synchronously on first query (strict
    /// loader chain, single on-demand load per process).
    pub fn new(
        artifact_root: &std::path::Path,
        actor_projection: ActorRoutingProjectionRef,
    ) -> Result<Self, ActorRoutingProjectionError> {
        let projection_store = ActorRoutingProjectionStore::open(artifact_root)?;
        Ok(Self {
            projection_store,
            actor_projection,
            catalog: Mutex::new(None),
            captures: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            loads: AtomicU64::new(0),
        })
    }

    /// Projection schema version (C-actor §7).
    pub fn schema_version(&self) -> &str {
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
    }

    /// The loaded catalog, loading it on demand through the strict
    /// projection store when it is not cached yet. `None` means the
    /// projection could not be loaded (record missing / malformed /
    /// non-canonical): queries fail closed.
    fn catalog(&self) -> Option<Arc<ActorRoutingCatalog>> {
        if let Some(catalog) = self
            .catalog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return Some(catalog);
        }
        self.load_catalog()
    }

    fn load_catalog(&self) -> Option<Arc<ActorRoutingCatalog>> {
        match self.projection_store.load_catalog(&self.actor_projection) {
            Ok(catalog) => {
                self.loads.fetch_add(1, Ordering::Relaxed);
                *self
                    .catalog
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&catalog));
                Some(catalog)
            }
            Err(_) => None,
        }
    }

    /// Reloads the projection record (build switches replace
    /// `actor-routing/current.json`; the cached catalog would otherwise stay
    /// stale for the process lifetime and every actor lookup of the new build
    /// would fail closed). The retry happens at most once per query.
    fn reload_catalog(&self) -> Option<Arc<ActorRoutingCatalog>> {
        *self
            .catalog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.load_catalog()
    }

    /// Exact typed-key hit test.
    pub fn has_method(&self, query: &CatalogQuery) -> bool {
        self.method_for(query).is_some()
    }

    /// Exact typed-key lookup returning the immutable method entry with its
    /// deployment/package binding. A cache miss reloads the projection once
    /// (build switches replace the record) and retries before failing.
    pub fn method_for(&self, query: &CatalogQuery) -> Option<ActorRoutingMethod> {
        if let Some(found) = self.method_for_with(&self.catalog(), query) {
            return Some(found);
        }
        self.method_for_with(&self.reload_catalog(), query)
    }

    fn method_for_with(
        &self,
        catalog: &Option<Arc<ActorRoutingCatalog>>,
        query: &CatalogQuery,
    ) -> Option<ActorRoutingMethod> {
        self.captures.fetch_add(1, Ordering::Relaxed);
        let Some(catalog) = catalog.as_ref() else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let found = catalog
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

    /// Deployment build id of the actor implementation (any method entry of
    /// the actor; owner-control/eviction route anchoring, M4). `None` when
    /// the catalog is not loaded or the actor is absent. A cache miss
    /// reloads the projection once and retries (build-switch staleness).
    pub fn deployment_build_id_for(
        &self,
        service_id: &str,
        actor_abi_identity: &ActorAbiIdentity,
        actor_implementation_identity: &ActorImplementationIdentity,
    ) -> Option<String> {
        if let Some(build_id) = self.deployment_build_id_for_with(
            &self.catalog(),
            service_id,
            actor_abi_identity,
            actor_implementation_identity,
        ) {
            return Some(build_id);
        }
        self.deployment_build_id_for_with(
            &self.reload_catalog(),
            service_id,
            actor_abi_identity,
            actor_implementation_identity,
        )
    }

    fn deployment_build_id_for_with(
        &self,
        catalog: &Option<Arc<ActorRoutingCatalog>>,
        service_id: &str,
        actor_abi_identity: &ActorAbiIdentity,
        actor_implementation_identity: &ActorImplementationIdentity,
    ) -> Option<String> {
        self.captures.fetch_add(1, Ordering::Relaxed);
        let catalog = catalog.as_ref()?;
        let found = catalog.entries().iter().find(|entry| {
            entry.actor.service_id == service_id
                && &entry.actor.actor_abi_identity == actor_abi_identity
                && &entry.actor_implementation_identity == actor_implementation_identity
        });
        let build_id = found.map(|entry| entry.deployment.deployment_artifact_identity.to_string());
        if build_id.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        build_id
    }

    pub fn health(&self) -> CatalogHealth {
        CatalogHealth {
            captures: self.captures.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    /// Public access to the lazily loaded catalog (owner-resolution reads;
    /// M4). Triggers the on-demand load when not cached.
    pub fn catalog_snapshot(&self) -> Option<Arc<ActorRoutingCatalog>> {
        self.catalog()
    }

    /// Total on-demand projection loads (health diagnostics; a view performs
    /// at most one successful load).
    pub fn loads(&self) -> u64 {
        self.loads.load(Ordering::Relaxed)
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

//! Immutable actor method index built once by the artifact loader.

use std::sync::Arc;

use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
};

/// Immutable, source-free actor method catalog over one routing projection.
///
/// The projection construction already guarantees sorted, unique full-key
/// entries; this catalog exposes exact full-key lookup, actor-scoped entries
/// and unique actor refs without establishing an independent index or refresh
/// (plan §3.2 / §3.3). Admission / owner-control query semantics belong to
/// C-actor / W-actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRoutingCatalog {
    projection: Arc<ActorRoutingProjection>,
}

impl ActorRoutingCatalog {
    pub fn from_projection(projection: Arc<ActorRoutingProjection>) -> Self {
        Self { projection }
    }

    pub fn projection(&self) -> &ActorRoutingProjection {
        &self.projection
    }

    /// Sorted entries of the immutable projection.
    pub fn entries(&self) -> &[ActorRoutingMethod] {
        &self.projection.methods
    }

    pub fn len(&self) -> usize {
        self.projection.methods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projection.methods.is_empty()
    }

    /// Exact lookup by the full typed method key.
    pub fn get(&self, key: &ActorRoutingMethod) -> Option<&ActorRoutingMethod> {
        self.projection
            .methods
            .binary_search(key)
            .ok()
            .map(|index| &self.projection.methods[index])
    }

    pub fn contains(&self, key: &ActorRoutingMethod) -> bool {
        self.get(key).is_some()
    }

    /// All entries owned by one stable actor ref (entries are grouped by the
    /// full-key sort, so this is a range over the sorted projection).
    pub fn methods_for_actor<'a>(
        &'a self,
        actor: &'a ActorRoutingRef,
    ) -> impl Iterator<Item = &'a ActorRoutingMethod> + 'a {
        let actor = actor.clone();
        self.projection
            .methods
            .iter()
            .filter(move |method| method.actor == actor)
    }

    /// Unique stable actor refs in projection order.
    pub fn actor_refs(&self) -> impl Iterator<Item = &ActorRoutingRef> {
        let mut previous: Option<&ActorRoutingRef> = None;
        self.projection.methods.iter().filter_map(move |method| {
            let actor = &method.actor;
            if previous.is_some_and(|seen| seen == actor) {
                None
            } else {
                previous = Some(actor);
                Some(actor)
            }
        })
    }
}

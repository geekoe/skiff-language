//! Strict reader/loader for the A0-frozen actor routing projection.
//!
//! This module is the Router Rust consumer boundary for
//! `skiff-deployment::projection::actor_routing` (A0 contract §2.4). It reads
//! the source-free projection record with the same strict chain used by the
//! deployment ecosystem store (bounded read, duplicate-key-free strict JSON,
//! exact schema version, typed deserialization with `deny_unknown_fields` and
//! construction invariants, canonical bytes), and it never reads
//! PackageArtifact, File IR, source or executable payloads.
//!
//! The canonical projection record identity / path derivation is the A1
//! producer output surface and is not frozen by A0; `ActorRoutingProjectionRef`
//! intentionally carries only an escape-proof relative record path and will be
//! aligned with A1 / contracts-bootstrap at merge time (A3 leaf D2).

mod actor_routing;
mod catalog;
mod strict_json;

pub use actor_routing::{
    ActorRoutingProjectionError, ActorRoutingProjectionRef, ActorRoutingProjectionStore,
    MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES,
};
pub use catalog::ActorRoutingCatalog;
pub use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

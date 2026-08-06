//! W-bootstrap: the M4 bootstrap assembly (artifact store + profile
//! validation), the bounded blocking loader pool, and the strict actor
//! routing projection loader consumed on demand by the actor catalog view.
//!
//! M4 removes the committed/pending matrix, the Mongo activation repository
//! and the `RoutingEpoch`: bootstrap loads no full deployment state and the
//! fail-closed set is exactly store-open failure + invalid profile.

mod assembly;
mod loader;
mod strict_loader;

pub use assembly::{
    BootstrapAssemblyError, RouterBootstrapAssembly, ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};
pub use loader::{
    BlockingLoader, BlockingLoaderError, BlockingLoaderHealth, BlockingLoaderOptions,
};
pub use strict_loader::{BootstrapLoadFailure, BootstrapStrictLoader};

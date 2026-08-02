//! W-bootstrap: committed bootstrap reader, strict epoch loader, bounded
//! blocking loader pool, and the single-authority `ActiveRoutingEpochStore`.
//!
//! Frozen contracts consumed here:
//! `doc/implementation/router-rust-migration-c-bootstrap-contract.md`,
//! `...-c-model-artifact-contract.md`, `...-c-model-bootstrap-wire-contract.md`
//! (authority design plan §3.3/§3.8/§5.4/§7 E-bootstrap). The reader consumes
//! the W-activation-state `ActivationStateRepository` read side; the strict
//! loader consumes the A3 artifact stores; the epoch source is wired into the
//! W-session `SessionLayer` seam.

mod epoch;
mod loader;
mod reader;
mod runner;
mod strict_loader;

pub use epoch::{ActiveRoutingEpochStore, EpochStoreHealth, RoutingEpoch, RoutingEpochHealth};
pub use loader::{
    BlockingLoader, BlockingLoaderError, BlockingLoaderHealth, BlockingLoaderOptions,
};
pub use reader::{
    BootstrapReadOutcome, CanonicalCommittedRefValidator, CommittedActivationBootstrapReader,
    CommittedBootstrapRefs, CommittedRefValidator, ReaderFailClosedCounters,
};
pub use runner::{BootstrapError, BootstrapHealthSnapshot, BootstrapRunner};
pub use strict_loader::{BootstrapLoadFailure, BootstrapStrictLoader};

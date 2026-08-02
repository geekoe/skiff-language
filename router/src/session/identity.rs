//! Typed connection/session identity and the registered assembly tuple
//! (authority design §3.2/§3.4, C-session contract §2).
//!
//! `RuntimeConnectionEpoch` and `RuntimeSessionEpoch` are not interchangeable:
//! a connection epoch exists from accept until capabilities bind, while a
//! session epoch exists only after replica identity is fixed on the same
//! physical connection. Reconnect is always a new connection epoch and a new
//! session epoch, even for the same `replica_id`.

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};

/// Physical connection identity before replica binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeConnectionEpoch {
    pub opaque_connection_id: String,
    pub generation: u64,
}

/// Logical Runtime session identity, bound on the same physical connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeSessionEpoch {
    pub replica_id: String,
    pub connection_generation: u64,
}

/// Exact registered assembly tuple owned by `RuntimeRegistrationDirectory`
/// (C-session contract §2). The same shape is the committed routing epoch
/// captured by `RuntimeRegistrationTransition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAssemblyTuple {
    pub environment: String,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

impl RegisteredAssemblyTuple {
    pub fn assembly_identity(&self) -> &str {
        self.assembly.assembly_identity.as_str()
    }

    pub fn snapshot_id(&self) -> &str {
        self.config_snapshot.snapshot_id.as_str()
    }
}

/// Committed epoch tuple captured by a registration transition. This is the
/// W-session seam for the bootstrap lane's `ActiveRoutingEpochStore`: when the
/// store lands, it supplies this tuple (and later a pending epoch) without
/// changing the session module boundary.
pub type CommittedEpoch = RegisteredAssemblyTuple;

/// A routable registration revision (one complete revision, never a mixture
/// of old and new tuple fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutableRevision {
    pub session_epoch: RuntimeSessionEpoch,
    pub registered_tuple: RegisteredAssemblyTuple,
    pub revision: u64,
}

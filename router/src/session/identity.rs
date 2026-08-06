//! Typed connection/session identity (authority design §3.2/§3.4,
//! C-session contract §2).
//!
//! `RuntimeConnectionEpoch` and `RuntimeSessionEpoch` are not interchangeable:
//! a connection epoch exists from accept until capabilities bind, while a
//! session epoch exists only after replica identity is fixed on the same
//! physical connection. Reconnect is always a new connection epoch and a new
//! session epoch, even for the same `replica_id`.
//!
//! M4: registration is capabilities-only; there is no registered assembly
//! tuple and no epoch identity on the wire.

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

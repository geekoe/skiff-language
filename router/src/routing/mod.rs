//! W-routing-query: stateless exact candidate projection
//! (`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-routing-query-contract.md`,
//! authority design plan §3.3/§5.4).
//!
//! `RuntimeCandidateQuery` is stateless and side-effect free: the candidate
//! set is decided only by one captured `Arc<RoutingEpoch>` and the caller's
//! typed directory view (exact registered tuple / registration revision /
//! cancellation / dispatch capabilities). It owns no index, no refresh, no
//! cache and no heartbeat/health input; heartbeat freshness never affects
//! admission. The same corpus
//! (`runtime/transport/testdata/routing-query/scenarios/`) is consumed by
//! W-routing-query and will be reused by W-dispatch/W-activation.

pub mod query;

pub use query::{
    CandidateDirectoryView, CandidateQuery, CandidateSession, DispatchCapabilities, DispatchMode,
    RegisteredSessionLease, RoutingQueryCounters, RuntimeCandidateQuery, SessionCancellation,
};

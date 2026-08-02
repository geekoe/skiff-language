//! W-dispatch: `RuntimeAdmissionPool` + `RequestDispatcher`
//! (authority design §3.2/§3.3, C-dispatch + C-routing-query +
//! C-model-request).
//!
//! Frozen contract consumed here:
//! `doc/implementation/router-rust-migration-c-dispatch-contract.md` and its
//! same-chain contracts (`c-routing-query`, `c-model-request`, `c-session`).
//! The admission pipeline is:
//!
//! ```text
//! capture RoutingEpoch -> CandidateQuery leases -> select + reserve permit
//!   -> revalidate (epoch/revision/tuple/cancellation)
//!   -> enqueue (failure releases and reselects / fail closed)
//!   -> terminal releases the permit exactly once
//! ```
//!
//! Owner boundary (§3.2): `RuntimeAdmissionPool` owns per-session capacity
//! permits and the selection cursor/policy only; `RequestDispatcher` owns
//! ordinary unary/stream pending, terminal and derived function-spawn
//! correlation only. Neither owns session truth, sockets or the active
//! routing epoch: epoch capture and candidate projection are consumed through
//! the typed ports in [`candidate`], and session cancellation arrives through
//! `on_session_closed` (the C-session barrier terminal).
//!
//! Same-wave seam: W-routing-query owns the stateless exact candidate
//! projection; this module defines the dispatch-side typed port
//! (`CandidateQuery`/`RegisteredSessionLease`/`LeaseRevalidate`) per the
//! contracts-request chain and drives it with fake implementations in tests.
//! Integration aligns the real W-routing-query port with this seam.

mod admission;
mod candidate;
mod dispatcher;
mod frame;
mod health;
mod types;

pub use admission::{AdmissionCounters, Permit, PermitLedger, Reservation, RuntimeAdmissionPool};
pub use candidate::{
    CandidateQuery, CandidateQueryInput, DispatchCapabilities, DispatchMode, LeaseRevalidate,
    RegisteredSessionLease, RevalidateOutcome, RoutingEpochSource, ServiceDeploymentQuery,
};
pub use dispatcher::{
    CancelFrame, DispatchedFrame, FrameOutcome, PendingTerminal, RequestDispatcher, RequestOutcome,
    RuntimeDispatcherOptions, SpawnRejectReason, SpawnSubmitResult, SubmitRejectReason,
    SubmitResult,
};
pub use frame::{
    ActorMethodSpawnControl, RuntimePeer, RuntimeResponseFrame, SessionAbortControl, TimeoutCheck,
    WireTimeoutCheck,
};
pub use health::{
    AdmissionHealth, DispatcherHealthSnapshot, PendingHealth, SpawnHealth, TerminalHealth,
    TerminalSource,
};
pub use types::{
    derived_deadline, ActorMethodSpawnDispatch, DerivedSpawnResult, DispatchRequest,
    RequestAuthority, RequestDeadline, SpawnSubmit, SpawnTargetKind,
};

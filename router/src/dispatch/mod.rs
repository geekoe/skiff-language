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
//! capture RoutingEpoch -> RuntimeCandidateQuery leases -> select + reserve permit
//!   -> revalidate (epoch/revision/tuple/cancellation)
//!   -> enqueue (failure releases and reselects / fail closed)
//!   -> terminal releases the permit exactly once
//! ```
//!
//! Owner boundary (§3.2): `RuntimeAdmissionPool` owns per-session capacity
//! permits and the selection cursor/policy only; `RequestDispatcher` owns
//! ordinary unary/stream pending, terminal and derived function-task
//! correlation only. Neither owns session truth, sockets or the active
//! routing epoch: epoch capture and candidate projection are consumed through
//! the typed ports in [`candidate`], and session cancellation arrives through
//! `on_session_closed` (the C-session barrier terminal).
//!
//! Same-wave alignment: W-routing-query owns the stateless exact candidate
//! projection (`RuntimeCandidateQuery`) and the canonical typed surface
//! (`CandidateQuery`, `RegisteredSessionLease`, `SessionCancellation`,
//! `DispatchCapabilities`, `DispatchMode`); dispatch consumes them through
//! the narrow adapter in [`candidate`] and defines only the ports without a
//! routing counterpart (`CandidateViewSource`, `LeaseRevalidate`,
//! `RoutingEpochSource`). No candidate/lease type is duplicated.

mod admission;
mod candidate;
mod dispatcher;
mod frame;
mod health;
mod types;

pub use admission::{AdmissionCounters, Permit, PermitLedger, Reservation, RuntimeAdmissionPool};
pub use candidate::{
    candidate_query_from_request, capabilities_from_wire_names, dispatch_mode_as_str,
    dispatch_mode_from_wire, CandidateViewSource, LeaseRevalidate, RevalidateOutcome,
    RoutingEpochSource,
};
pub use dispatcher::{
    CancelFrame, DispatchedFrame, FrameOutcome, PendingTerminal, RequestDispatcher, RequestOutcome,
    RuntimeDispatcherOptions, SubmitRejectReason, SubmitResult, TaskAttemptSubmitResult,
};
pub use frame::{
    NoopTaskAttemptTerminalSink, RuntimePeer, RuntimeResponseFrame, SessionAbortControl,
    TaskAttemptTerminalOutcome, TaskAttemptTerminalSink, TimeoutCheck, WireTimeoutCheck,
};
pub use health::{
    AdmissionHealth, DispatcherHealthSnapshot, PendingHealth, TaskHealth, TerminalHealth,
    TerminalSource,
};
pub use types::{DispatchSubmit, RequestAuthority, RequestDeadline, TaskAttemptSubmit};

//! Dispatch-side adapter over the canonical W-routing-query types
//! (C-routing-query §2/§5, plan §3.3 steps 1-5).
//!
//! W-routing-query owns the stateless exact candidate projection
//! (`RuntimeCandidateQuery`) and the canonical typed surface
//! (`CandidateQuery`, `CandidateDirectoryView`, `RegisteredSessionLease`,
//! `SessionCancellation`, `DispatchCapabilities`, `DispatchMode`). This
//! module is the narrow adapter layer: it imports the canonical types for
//! dispatch consumers, maps wire surfaces (`request.start` mode / capability
//! names) onto the canonical types, and defines the two dispatch-owned ports
//! that have no routing counterpart: the directory view source (C-dispatch
//! §3 step 2) and the enqueue-time lease revalidation (step 5).

use std::fmt;

use skiff_runtime_transport::protocol::RuntimeDispatchModeCapability;

use crate::routing::{
    CandidateDirectoryView, CandidateQuery, DispatchCapabilities, DispatchMode,
    RegisteredSessionLease,
};

use super::types::DispatchSubmit;

/// Wire `request.start` mode mapping (C-model-request §3.1). The codec already
/// rejects other values; this stays fallible so the dispatcher can fail
/// closed.
pub fn dispatch_mode_from_wire(mode: &str) -> Option<DispatchMode> {
    match mode {
        "unary" => Some(DispatchMode::Unary),
        "serverStream" => Some(DispatchMode::ServerStream),
        _ => None,
    }
}

/// Wire mode string projection (mirrors the canonical serde rename).
pub fn dispatch_mode_as_str(mode: DispatchMode) -> &'static str {
    match mode {
        DispatchMode::Unary => "unary",
        DispatchMode::ServerStream => "serverStream",
    }
}

/// Wire capability name list (`"unary"` / `"serverStream"`) mapped onto the
/// canonical query-side capability bitmap (C-routing-query §2.1).
pub fn capabilities_from_wire_names(names: &[String]) -> DispatchCapabilities {
    DispatchCapabilities::from_dispatch_modes(names.iter().filter_map(|name| match name.as_str() {
        "unary" => Some(RuntimeDispatchModeCapability::Unary),
        "serverStream" => Some(RuntimeDispatchModeCapability::ServerStream),
        _ => None,
    }))
}

/// Builds the canonical query input for one admission from the validated
/// `request.start` header (integration-contract-v2 §1: mode + routing
/// buildId). A header without a routing buildId fails closed (no candidate).
pub fn candidate_query_from_request(request: &DispatchSubmit) -> Option<CandidateQuery> {
    candidate_query_from_build_id(request.mode(), request.header.routing.build_id.as_deref())
}

/// Builds the canonical query input from a dispatch mode and an optional
/// routing buildId (task-attempt and WS-connect admissions use the same
/// helper; `None` buildId fails closed).
pub fn candidate_query_from_build_id(
    mode: DispatchMode,
    build_id: Option<&str>,
) -> Option<CandidateQuery> {
    build_id.map(|build_id| CandidateQuery {
        mode,
        build_id: build_id.to_string(),
    })
}

/// Directory view source consumed by the admission pipeline
/// (C-dispatch §3 step 2).
///
/// The production implementation snapshots the
/// `RuntimeRegistrationDirectory` under its lock and supplies the per-session
/// dispatch capabilities binding, then hands the coherent
/// [`CandidateDirectoryView`] to the canonical [`RuntimeCandidateQuery`].
/// This seam keeps session truth outside the dispatcher owner.
pub trait CandidateViewSource: Send + Sync + fmt::Debug {
    fn view(&self) -> CandidateDirectoryView;
}

/// Atomic revalidation immediately before enqueue (plan §3.3 step 5).
///
/// Re-checks session epoch, registration revision, exact registered tuple and
/// cancellation against the directory. `request_id` is passed for fake
/// injection; real implementations ignore it.
pub trait LeaseRevalidate: Send + Sync + fmt::Debug {
    fn revalidate(&self, request_id: &str, lease: &RegisteredSessionLease) -> RevalidateOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidateOutcome {
    Ok,
    Cancelled,
    StaleRevision,
    TupleMismatch,
}

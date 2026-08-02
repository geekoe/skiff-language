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
use std::sync::Arc;

use skiff_runtime_transport::protocol::RuntimeDispatchModeCapability;

use crate::bootstrap::RoutingEpoch;
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
/// `request.start` header (C-routing-query §2.1: mode + exact deployment).
pub fn candidate_query_from_request(request: &DispatchSubmit) -> CandidateQuery {
    CandidateQuery {
        mode: request.mode(),
        deployment: request.header.routing.deployment.clone(),
    }
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

/// Captured routing epoch source (plan §3.3 step 1).
///
/// W-bootstrap seam: the production implementation wraps
/// `Arc<ActiveRoutingEpochStore>` and captures the current whole epoch.
/// `None` means no epoch published yet: admission fails closed.
pub trait RoutingEpochSource: Send + Sync + fmt::Debug {
    fn capture(&self) -> Option<Arc<RoutingEpoch>>;
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
